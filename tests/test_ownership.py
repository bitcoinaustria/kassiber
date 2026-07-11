"""Pure-engine tests for kassiber.core.ownership (no subprocess, no network)."""

import json
import sqlite3
import unittest
import unittest.mock

from kassiber.core import ownership
from kassiber.core.ownership import OwnedIndex, OwnedMatch


def _engine_conn():
    """Minimal in-memory DB with just the tables the engine reads."""
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.executescript(
        """
        CREATE TABLE accounts (id TEXT, code TEXT, label TEXT);
        CREATE TABLE wallets (
            id TEXT, profile_id TEXT, label TEXT, kind TEXT,
            config_json TEXT, account_id TEXT
        );
        CREATE TABLE wallet_utxos (
            profile_id TEXT, wallet_id TEXT, txid TEXT, vout INTEGER,
            address TEXT, branch_label TEXT, branch_index INTEGER,
            address_index INTEGER, chain TEXT, network TEXT
        );
        CREATE TABLE transactions (
            profile_id TEXT, wallet_id TEXT, external_id TEXT, raw_json TEXT
        );
        """
    )
    return conn


def _match(label="Vault", branch="receive", index=0, source="derived", **kw):
    return OwnedMatch(
        wallet_id=kw.get("wallet_id", "w1"),
        wallet_label=label,
        account=kw.get("account", "treasury"),
        chain=kw.get("chain", "bitcoin"),
        network=kw.get("network", "main"),
        branch_label=branch,
        address_index=index,
        derivation_path=kw.get("derivation_path", f"m/84'/0'/0'/0/{index}"),
        source=source,
        wallet_kind=kw.get("wallet_kind", ""),
    )


class ClassifyTokenTypeTests(unittest.TestCase):
    def test_64_hex_is_txid(self):
        self.assertEqual(ownership.classify_token_type("a" * 64), "txid")
        self.assertEqual(ownership.classify_token_type("00" * 32), "txid")

    def test_address_is_address(self):
        self.assertEqual(
            ownership.classify_token_type("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"),
            "address",
        )

    def test_64_non_hex_is_address(self):
        self.assertEqual(ownership.classify_token_type("z" * 64), "address")


class ParseTokensTests(unittest.TestCase):
    def test_forced_types_and_autodetect(self):
        parsed, invalid = ownership.parse_tokens(
            addresses=["bc1qaddr"],
            txids=["ab" * 32],
            candidates=["cd" * 32, "bc1qcandidate"],
        )
        self.assertEqual(invalid, [])
        kinds = {(p["type"], p["normalized"]) for p in parsed}
        self.assertIn(("address", "bc1qaddr"), kinds)
        self.assertIn(("txid", "ab" * 32), kinds)
        self.assertIn(("txid", "cd" * 32), kinds)
        self.assertIn(("address", "bc1qcandidate"), kinds)

    def test_dedup_and_comments_and_blanks(self):
        parsed, invalid = ownership.parse_tokens(
            file_text="bc1qaddr\n# a comment\n\nbc1qaddr\n",
        )
        self.assertEqual(invalid, [])
        self.assertEqual(len(parsed), 1)
        self.assertEqual(parsed[0]["normalized"], "bc1qaddr")

    def test_invalid_forced_txid(self):
        parsed, invalid = ownership.parse_tokens(txids=["not-a-real-txid"])
        self.assertEqual(parsed, [])
        self.assertEqual(len(invalid), 1)
        self.assertEqual(invalid[0]["type"], "txid")


class OwnedIndexPhysicalScopeTests(unittest.TestCase):
    def test_same_outpoint_and_txid_are_separate_across_networks(self):
        index = OwnedIndex()
        txid = "ab" * 32
        main_match = _match(label="Main", wallet_id="main", network="main")
        regtest_match = _match(
            label="Regtest", wallet_id="regtest", network="regtest"
        )
        index.add_outpoint(txid, 0, main_match)
        index.add_outpoint(txid, 0, regtest_match)
        index.note_txid(
            txid, "main", "Main", chain="bitcoin", network="main"
        )
        index.note_txid(
            txid, "regtest", "Regtest", chain="bitcoin", network="regtest"
        )

        self.assertEqual(
            [match.wallet_id for match in index.lookup_outpoint(
                f"{txid}:0", chain="bitcoin", network="main"
            )],
            ["main"],
        )
        self.assertEqual(
            [match.wallet_id for match in index.lookup_outpoint(
                f"{txid}:0", chain="bitcoin", network="regtest"
            )],
            ["regtest"],
        )
        self.assertEqual(
            index.lookup_txid_wallets(txid, chain="bitcoin", network="main"),
            {("main", "Main")},
        )
        self.assertEqual(
            index.lookup_txid_wallets(
                txid, chain="bitcoin", network="regtest"
            ),
            {("regtest", "Regtest")},
        )


class ClassifyAddressTests(unittest.TestCase):
    def test_owned_address_match(self):
        index = OwnedIndex()
        index.add_address("bc1qOWNED", _match(branch="change", index=7))
        result = ownership.classify_address(
            {"input": "bc1qowned", "type": "address", "chain": "bitcoin"}, index
        )
        self.assertEqual(result["status"], "owned")
        self.assertEqual(result["classification"], "owned_address")
        self.assertEqual(result["matches"][0]["branch"], "change")
        self.assertIn("change #7", result["note"])

    def test_external_address(self):
        index = OwnedIndex()
        result = ownership.classify_address(
            {"input": "bc1qexternal", "type": "address", "chain": "bitcoin"}, index
        )
        self.assertEqual(result["status"], "external")
        self.assertEqual(result["matches"], [])

    def test_multiple_owners_noted(self):
        index = OwnedIndex()
        index.add_address("bc1qshared", _match(label="WalletA"))
        index.add_address("bc1qshared", _match(label="WalletB", wallet_id="w2"))
        result = ownership.classify_address(
            {"input": "bc1qshared", "type": "address", "chain": "bitcoin"}, index
        )
        self.assertEqual(result["status"], "owned")
        self.assertTrue(result["ownership_ambiguous"])
        self.assertIn("Also matches", result["note"])

    def test_descriptor_match_is_canonical_over_address_list(self):
        index = OwnedIndex()
        index.add_address(
            "bc1qshared",
            _match(
                label="Address list",
                wallet_id="addr",
                source="address_list",
                wallet_kind="address",
            ),
        )
        index.add_address(
            "bc1qshared",
            _match(
                label="Trezor",
                wallet_id="trezor",
                source="derived",
                wallet_kind="xpub",
            ),
        )

        result = ownership.classify_address(
            {"input": "bc1qshared", "type": "address", "chain": "bitcoin"}, index
        )

        self.assertEqual(result["canonical_wallet"], "Trezor")
        self.assertEqual(result["canonical_wallet_id"], "trezor")
        self.assertEqual(result["matches"][0]["wallet"], "Trezor")
        self.assertTrue(result["ownership_ambiguous"])


class ClassifyTxidTests(unittest.TestCase):
    def _index(self):
        index = OwnedIndex()
        index.by_outpoint["aa" * 32 + ":0"] = _match(branch="change", index=3)
        index.add_script("00146f6e656473637269707400000000000000000000", _match())
        return index

    def test_self_transfer(self):
        index = self._index()
        legs = {
            "inputs": [{"outpoint": "aa" * 32 + ":0", "script": None}],
            "outputs": [{"n": 0, "script": "00146f6e656473637269707400000000000000000000"}],
            "chain": "bitcoin",
            "source": "chain",
        }
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertEqual(result["classification"], "self_transfer")
        self.assertEqual(result["owned_inputs"], 1)
        self.assertEqual(result["external_outputs"], 0)

    def test_outbound_payment_with_change(self):
        index = self._index()
        legs = {
            "inputs": [{"outpoint": "aa" * 32 + ":0", "script": None}],
            "outputs": [
                {"n": 0, "script": "00146f6e656473637269707400000000000000000000"},
                {"n": 1, "script": "0014deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"},
            ],
            "chain": "bitcoin",
            "source": "chain",
        }
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertEqual(result["classification"], "outbound_payment")
        self.assertEqual(result["external_outputs"], 1)

    def test_inbound_receipt(self):
        index = self._index()
        legs = {
            "inputs": [{"outpoint": "ff" * 32 + ":9", "script": "0014external"}],
            "outputs": [{"n": 0, "script": "00146f6e656473637269707400000000000000000000"}],
            "chain": "bitcoin",
            "source": "chain",
        }
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertEqual(result["classification"], "inbound_receipt")
        self.assertEqual(result["owned_inputs"], 0)

    def test_ambiguous_output_counts_once_and_names_canonical_wallet(self):
        index = OwnedIndex()
        shared_script = "00146f6e656473637269707400000000000000000000"
        index.add_script(
            shared_script,
            _match(
                label="Address list",
                wallet_id="addr",
                source="address_list",
                wallet_kind="address",
            ),
        )
        index.add_script(
            shared_script,
            _match(
                label="Trezor",
                wallet_id="trezor",
                source="derived",
                wallet_kind="xpub",
            ),
        )
        legs = {
            "inputs": [{"outpoint": "ff" * 32 + ":9", "script": "0014external"}],
            "outputs": [{"n": 0, "script": shared_script}],
            "chain": "bitcoin",
            "source": "chain",
        }

        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )

        self.assertEqual(result["classification"], "inbound_receipt")
        self.assertEqual(result["owned_outputs"], 1)
        self.assertTrue(result["ownership_ambiguous"])
        self.assertEqual(result["ambiguous_legs"], 1)
        output_leg = result["legs"][1]
        self.assertEqual(output_leg["wallet"], "Trezor")
        self.assertEqual(output_leg["wallets"], ["Trezor", "Address list"])

    def test_external_transaction(self):
        index = self._index()
        legs = {
            "inputs": [{"outpoint": "ff" * 32 + ":9", "script": "0014external"}],
            "outputs": [{"n": 0, "script": "0014deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"}],
            "chain": "bitcoin",
            "source": "chain",
        }
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertEqual(result["status"], "external")
        self.assertEqual(result["classification"], "external")

    def test_script_matches_are_scoped_to_local_tx_network(self):
        index = OwnedIndex()
        script = "00146f6e656473637269707400000000000000000000"
        index.add_script(script, _match(label="Main Wallet", network="main"))
        index.add_script(script, _match(label="Regtest Wallet", network="regtest"))
        legs = {
            "inputs": [{"outpoint": "ff" * 32 + ":9", "script": "0014external"}],
            "outputs": [{"n": 0, "script": script}],
            "chain": "bitcoin",
            "network": "regtest",
            "source": "local_tx",
        }

        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )

        self.assertEqual(result["classification"], "inbound_receipt")
        self.assertEqual(result["wallets"], ["Regtest Wallet"])
        output_leg = result["legs"][1]
        self.assertEqual(output_leg["wallets"], ["Regtest Wallet"])

    def test_no_legs_but_local_txid(self):
        index = OwnedIndex()
        index.note_txid("cc" * 32, "w1", "Vault")
        result = ownership.classify_txid(
            {"input": "cc" * 32, "normalized": "cc" * 32, "type": "txid"}, index, None
        )
        self.assertEqual(result["status"], "owned")
        self.assertEqual(result["classification"], "touches_wallet")
        self.assertIn("Vault", result["wallets"])

    def test_no_legs_unknown(self):
        index = OwnedIndex()
        result = ownership.classify_txid(
            {"input": "dd" * 32, "normalized": "dd" * 32, "type": "txid"}, index, None
        )
        self.assertEqual(result["status"], "unknown")


class LocalTxLegsTests(unittest.TestCase):
    def test_legs_from_esplora_shape(self):
        raw = json.dumps(
            {
                "vin": [
                    {
                        "txid": "AA" * 32,
                        "vout": 0,
                        "prevout": {"scriptpubkey": "0014aa"},
                    }
                ],
                "vout": [
                    {"scriptpubkey": "0014bb"},
                    {"scriptpubkey": "0014cc"},
                ],
            }
        )
        legs = ownership._legs_from_local_tx_json(raw)
        self.assertEqual(legs["chain"], "bitcoin")
        self.assertEqual(legs["inputs"][0]["outpoint"], f"{'aa' * 32}:0")
        self.assertEqual(legs["inputs"][0]["script"], "0014aa")
        self.assertEqual([o["script"] for o in legs["outputs"]], ["0014bb", "0014cc"])

    def test_legs_from_electrum_decode_shape(self):
        # Electrum-synced raw_json uses script_hex on outputs and has no inline
        # prevout script; output ownership must still resolve.
        raw = json.dumps(
            {
                "vin": [
                    {"txid": "AA" * 32, "vout": 1, "sequence": 4_294_967_295}
                ],
                "vout": [
                    {"n": 0, "value": 1, "script_hex": "0014bb"},
                    {"n": 1, "value": 2, "script_hex": "0014cc"},
                ],
            }
        )
        legs = ownership._legs_from_local_tx_json(raw)
        self.assertEqual(legs["inputs"][0]["outpoint"], f"{'aa' * 32}:1")
        self.assertIsNone(legs["inputs"][0]["script"])
        self.assertEqual([o["script"] for o in legs["outputs"]], ["0014bb", "0014cc"])

    def test_malformed_prevout_txid_is_not_promoted_to_physical_identity(self):
        legs = ownership._legs_from_local_tx_json(
            json.dumps(
                {
                    "vin": [
                        {
                            "txid": "AA",
                            "vout": 0,
                            "prevout": {"scriptpubkey": "0014aa"},
                        }
                    ],
                    "vout": [{"n": 0, "scriptpubkey": "0014bb"}],
                }
            )
        )

        self.assertIsNone(legs["inputs"][0]["outpoint"])
        # Non-identity evidence remains available for conservative script lookup.
        self.assertEqual("0014aa", legs["inputs"][0]["script"])

    def test_electrum_shape_classifies_receipt_not_external(self):
        # Regression: the esplora-only parser read None scripts for Electrum
        # rows, misclassifying an owned receipt as external.
        index = OwnedIndex()
        index.add_script("0014bb", _match(branch="receive", index=0))
        legs = ownership._legs_from_local_tx_json(
            json.dumps(
                {
                    "vin": [{"txid": "AA" * 32, "vout": 1}],
                    "vout": [{"n": 0, "script_hex": "0014bb"}],
                }
            )
        )
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertEqual(result["status"], "owned")
        self.assertEqual(result["classification"], "inbound_receipt")
        self.assertEqual(result["owned_outputs"], 1)

    def test_local_tx_legs_include_wallet_network(self):
        conn = _engine_conn()
        conn.execute(
            "INSERT INTO wallets(id, profile_id, label, kind, config_json, account_id) "
            "VALUES(?,?,?,?,?,?)",
            (
                "w1",
                "p1",
                "Regtest Wallet",
                "address",
                '{"chain":"bitcoin","network":"regtest"}',
                None,
            ),
        )
        conn.execute(
            "INSERT INTO transactions(profile_id, wallet_id, external_id, raw_json) "
            "VALUES(?,?,?,?)",
            (
                "p1",
                "w1",
                "aa" * 32,
                '{"vin":[{"txid":"BB","vout":0}],"vout":[{"n":0,"script_hex":"0014bb"}]}',
            ),
        )

        legs = ownership.load_local_tx_legs(conn, "p1", "aa" * 32)

        self.assertIsNotNone(legs)
        self.assertEqual(legs["chain"], "bitcoin")
        self.assertEqual(legs["network"], "regtest")

    def test_legs_from_non_tx_returns_none(self):
        self.assertIsNone(ownership._legs_from_local_tx_json('{"component": {}}'))
        self.assertIsNone(ownership._legs_from_local_tx_json("not json"))


class LegEdgeCaseTests(unittest.TestCase):
    def test_liquid_fee_output_does_not_break_self_transfer(self):
        # The Liquid fee output carries an empty scriptPubKey; it must not count
        # as an external recipient and flip a self-transfer to outbound payment.
        index = OwnedIndex()
        index.by_outpoint["aa" * 32 + ":0"] = _match(chain="liquid", network="liquidv1")
        index.add_script(
            "0014owned00000000000000000000000000000000",
            _match(branch="change", chain="liquid", network="liquidv1"),
        )
        legs = {
            "inputs": [{"outpoint": "aa" * 32 + ":0", "script": None}],
            "outputs": [
                {"n": 0, "script": "0014owned00000000000000000000000000000000"},
                {"n": 1, "script": ""},  # Liquid fee output
            ],
            "chain": "liquid",
            "source": "chain",
        }
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertEqual(result["classification"], "self_transfer")
        self.assertEqual(result["external_outputs"], 0)

    def test_zero_output_does_not_assert_self_transfer(self):
        index = OwnedIndex()
        index.by_outpoint["aa" * 32 + ":0"] = _match()
        legs = {
            "inputs": [{"outpoint": "aa" * 32 + ":0", "script": None}],
            "outputs": [],
            "chain": "bitcoin",
            "source": "chain",
        }
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertNotEqual(result["classification"], "self_transfer")
        self.assertEqual(result["classification"], "undetermined")

    def test_op_return_output_does_not_break_self_transfer(self):
        index = OwnedIndex()
        index.by_outpoint["aa" * 32 + ":0"] = _match()
        index.add_script("0014owned00000000000000000000000000000000", _match(branch="change"))
        legs = {
            "inputs": [{"outpoint": "aa" * 32 + ":0", "script": None}],
            "outputs": [
                {"n": 0, "script": "0014owned00000000000000000000000000000000"},
                {"n": 1, "script": "6a146d656d6f"},
            ],
            "chain": "bitcoin",
            "source": "chain",
        }
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertEqual(result["classification"], "self_transfer")
        self.assertEqual(result["external_outputs"], 0)

    def test_unresolved_inputs_with_owned_output_are_undetermined(self):
        index = OwnedIndex()
        index.add_script("0014owned00000000000000000000000000000000", _match(branch="change"))
        legs = {
            "inputs": [{"outpoint": "ff" * 32 + ":9", "script": None}],
            "outputs": [{"n": 0, "script": "0014owned00000000000000000000000000000000"}],
            "chain": "bitcoin",
            "source": "chain",
        }
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertEqual(result["status"], "owned")
        self.assertEqual(result["classification"], "undetermined")

    def test_local_txid_stays_owned_when_verified_legs_do_not_match(self):
        index = OwnedIndex()
        index.note_txid("bb" * 32, "w1", "Vault")
        legs = {
            "inputs": [{"outpoint": "ff" * 32 + ":9", "script": "0014external"}],
            "outputs": [{"n": 0, "script": "0014deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"}],
            "chain": "bitcoin",
            "source": "chain",
        }
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertEqual(result["status"], "owned")
        self.assertEqual(result["classification"], "touches_wallet")
        self.assertEqual(result["wallets"], ["Vault"])


class ChainDetectAndDedupTests(unittest.TestCase):
    def test_detect_chain(self):
        self.assertEqual(ownership._detect_chain("lq1qexample"), "liquid")
        self.assertEqual(ownership._detect_chain("ex1qexample"), "liquid")
        self.assertEqual(ownership._detect_chain("bc1qexample"), "bitcoin")
        self.assertEqual(ownership._detect_chain("1someBase58"), "")

    def test_case_variant_bech32_dedup(self):
        parsed, invalid = ownership.parse_tokens(
            candidates=["BC1QABCDEFGH", "bc1qabcdefgh"]
        )
        self.assertEqual(invalid, [])
        self.assertEqual(len(parsed), 1)

    def test_malformed_address_is_invalid(self):
        parsed, invalid = ownership.parse_tokens(addresses=["bc1q has a space"])
        self.assertEqual(parsed, [])
        self.assertEqual(len(invalid), 1)
        self.assertEqual(invalid[0]["type"], "address")

    def test_liquid_confidential_address_owned_via_string_fallback(self):
        # _script_hex_for_address can't canonicalize a Liquid confidential
        # address, so ownership rides the address-string index.
        index = OwnedIndex()
        index.add_address("lq1qwconfidentialexampleaddress", _match(chain="liquid", network="liquidv1"))
        result = ownership.classify_address(
            {"input": "lq1qwconfidentialexampleaddress", "type": "address", "chain": "liquid"},
            index,
        )
        self.assertEqual(result["status"], "owned")


class BuildOwnedIndexTests(unittest.TestCase):
    def test_scan_to_index_is_inclusive_for_descriptor_derivation(self):
        conn = _engine_conn()
        conn.execute(
            "INSERT INTO wallets(id, profile_id, label, kind, config_json, account_id) "
            "VALUES(?,?,?,?,?,?)",
            ("w1", "p1", "Vault", "descriptor", '{"descriptor":"wpkh(xpub/.../*)"}', None),
        )
        wallet = conn.execute("SELECT * FROM wallets WHERE id = ?", ("w1",)).fetchone()

        class Plan:
            gap_limit = 0
            chain = "bitcoin"
            network = "main"

        with unittest.mock.patch.object(
            ownership, "load_wallet_descriptor_plan_from_config", return_value=Plan()
        ), unittest.mock.patch.object(
            ownership, "derive_descriptor_targets", return_value=[]
        ) as derive:
            ownership.build_owned_index(conn, "p1", [wallet], scan_to_index=5)

        self.assertEqual(derive.call_args.kwargs["end"], 6)

    def test_blank_address_metadata_is_stamped_bitcoin_main_in_index(self):
        conn = _engine_conn()
        conn.execute(
            "INSERT INTO wallets(id, profile_id, label, kind, config_json, account_id) "
            "VALUES(?,?,?,?,?,?)",
            ("w1", "p1", "Legacy", "address", '{"addresses":["legacy-owned"]}', None),
        )
        wallet = conn.execute("SELECT * FROM wallets WHERE id = ?", ("w1",)).fetchone()

        index, warnings = ownership.build_owned_index(
            conn, "p1", [wallet], derive=False
        )

        self.assertEqual(warnings, [])
        match = index.lookup_address("legacy-owned")[0]
        self.assertEqual((match.chain, match.network), ("bitcoin", "main"))

    def test_imported_txid_history_is_indexed_per_wallet_network(self):
        conn = _engine_conn()
        txid = "cd" * 32
        conn.executemany(
            "INSERT INTO wallets(id, profile_id, label, kind, config_json, account_id) "
            "VALUES(?,?,?,?,?,?)",
            [
                (
                    "main",
                    "p1",
                    "Main",
                    "address",
                    '{"chain":"bitcoin","network":"main"}',
                    None,
                ),
                (
                    "regtest",
                    "p1",
                    "Regtest",
                    "address",
                    '{"chain":"bitcoin","network":"regtest"}',
                    None,
                ),
            ],
        )
        conn.executemany(
            "INSERT INTO transactions(profile_id, wallet_id, external_id, raw_json) "
            "VALUES(?,?,?,?)",
            [
                ("p1", "main", txid, "{}"),
                ("p1", "regtest", txid, "{}"),
            ],
        )
        wallets = conn.execute(
            "SELECT w.*, NULL AS account_code, NULL AS account_label "
            "FROM wallets w ORDER BY id"
        ).fetchall()

        index, warnings = ownership.build_owned_index(
            conn, "p1", wallets, derive=False
        )

        self.assertEqual(warnings, [])
        self.assertEqual(
            index.lookup_txid_wallets(txid, chain="bitcoin", network="main"),
            {("main", "Main")},
        )
        self.assertEqual(
            index.lookup_txid_wallets(
                txid, chain="bitcoin", network="regtest"
            ),
            {("regtest", "Regtest")},
        )

    def test_wallet_config_can_raise_journal_ownership_scan_depth(self):
        conn = _engine_conn()
        conn.execute(
            "INSERT INTO wallets(id, profile_id, label, kind, config_json, account_id) "
            "VALUES(?,?,?,?,?,?)",
            (
                "w1",
                "p1",
                "Vault",
                "descriptor",
                '{"descriptor":"wpkh(xpub/.../*)","ownership_scan_to_index":750}',
                None,
            ),
        )
        wallet = conn.execute("SELECT * FROM wallets WHERE id = ?", ("w1",)).fetchone()

        class Plan:
            gap_limit = 0
            chain = "bitcoin"
            network = "main"

        with unittest.mock.patch.object(
            ownership, "load_wallet_descriptor_plan_from_config", return_value=Plan()
        ), unittest.mock.patch.object(
            ownership, "derive_descriptor_targets", return_value=[]
        ) as derive:
            ownership.build_owned_index(conn, "p1", [wallet], scan_to_index=500)

        self.assertEqual(derive.call_args.kwargs["end"], 751)

    def test_derivation_type_error_is_not_mislabeled_as_bad_history_floor(self):
        conn = _engine_conn()
        conn.execute(
            "INSERT INTO wallets(id, profile_id, label, kind, config_json, account_id) "
            "VALUES(?,?,?,?,?,?)",
            (
                "w1",
                "p1",
                "Vault",
                "descriptor",
                '{"descriptor":"wpkh(xpub/.../*)"}',
                None,
            ),
        )
        wallet = conn.execute("SELECT * FROM wallets WHERE id = ?", ("w1",)).fetchone()

        with unittest.mock.patch.object(
            ownership,
            "_derive_wallet_into_index",
            side_effect=TypeError("deriver bug"),
        ):
            with self.assertRaisesRegex(TypeError, "deriver bug"):
                ownership.build_owned_index(conn, "p1", [wallet])


class FlattenAndRedactTests(unittest.TestCase):
    def test_flatten_rows_share_keys(self):
        address_row = ownership.classify_address(
            {"input": "bc1qx", "type": "address", "chain": "bitcoin"}, OwnedIndex()
        )
        txid_row = ownership.classify_txid(
            {"input": "ee" * 32, "normalized": "ee" * 32, "type": "txid"},
            OwnedIndex(),
            None,
        )
        flat_addr = ownership.flatten_result_row(address_row)
        flat_txid = ownership.flatten_result_row(txid_row)
        self.assertEqual(set(flat_addr.keys()), set(flat_txid.keys()))

    def test_redact_drops_geometry(self):
        index = OwnedIndex()
        index.add_address("bc1qowned", _match(branch="change", index=12))
        owned = ownership.classify_address(
            {"input": "bc1qowned", "type": "address", "chain": "bitcoin"}, index
        )
        redacted = ownership.redact_result_for_ai(owned)
        self.assertNotIn("matches", redacted)
        self.assertNotIn("change #12", redacted["note"])
        self.assertEqual(redacted["wallets"], ["Vault"])
        self.assertEqual(redacted["status"], "owned")

    def test_redact_txid_drops_legs_and_geometry(self):
        # A txid result carries per-leg outpoints + branch labels; the AI
        # surface must not see any of it.
        index = OwnedIndex()
        index.by_outpoint["aa" * 32 + ":0"] = _match(branch="change", index=3)
        index.add_script("0014owned", _match())
        legs = {
            "inputs": [{"outpoint": "aa" * 32 + ":0", "script": None}],
            "outputs": [
                {"n": 0, "script": "0014owned"},
                {"n": 1, "script": "0014deadbeef"},
            ],
            "chain": "bitcoin",
            "source": "chain",
        }
        txid_result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertIn("legs", txid_result)  # full result has legs
        redacted = ownership.redact_result_for_ai(txid_result)
        self.assertNotIn("legs", redacted)
        self.assertNotIn("outpoint", json.dumps(redacted))
        self.assertNotIn("branch", json.dumps(redacted))
        self.assertEqual(redacted["status"], "owned")
        self.assertEqual(redacted["classification"], "outbound_payment")


class CsvHarvestTests(unittest.TestCase):
    BECH32 = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
    TXID = "a" * 64

    def test_headered_comma_csv_harvests_and_ignores_noise(self):
        text = f"date,address,amount,txid\n2024-01-01,{self.BECH32},0.5,{self.TXID}\n"
        got = ownership.extract_candidates_from_csv(text)
        self.assertIn(self.BECH32, got)
        self.assertIn(self.TXID, got)
        self.assertNotIn("0.5", got)
        self.assertNotIn("2024-01-01", got)

    def test_no_header_semicolon_content_harvest(self):
        text = f"{self.BECH32};100;done\nfoo;{self.TXID};x\n"
        got = ownership.extract_candidates_from_csv(text)
        self.assertEqual(sorted(got), sorted([self.BECH32, self.TXID]))

    def test_bom_and_tab_and_unrecognized_columns(self):
        text = f"\ufeffcol1\tcol2\nhello\t{self.BECH32}\n"
        got = ownership.extract_candidates_from_csv(text)
        self.assertEqual(got, [self.BECH32])

    def test_noise_only_returns_empty(self):
        self.assertEqual(
            ownership.extract_candidates_from_csv("name,amount\nalice,100\nbob,200\n"),
            [],
        )

    def test_plain_one_per_line_list(self):
        text = f"{self.BECH32}\n{self.TXID}\n# a comment, not an address\n"
        got = ownership.extract_candidates_from_csv(text)
        self.assertEqual(sorted(got), sorted([self.BECH32, self.TXID]))

    def test_identify_with_csv_text(self):
        conn = _engine_conn()
        report = ownership.identify(
            conn,
            "p1",
            csv_text=f"address,memo\n{self.BECH32},rent\n",
            scan_to_index=0,
        )
        inputs = [r["input"] for r in report["results"]]
        self.assertIn(self.BECH32, inputs)
        self.assertEqual(report["results"][0]["status"], "external")  # no wallets seeded

    def test_recognized_column_rejects_non_address_labels(self):
        # A recognized "wallet address" column with non-address labels (a common
        # exchange-CSV shape) must NOT promote "internal"/labels to candidates.
        text = (
            "type,wallet address,amount\n"
            "internal,internal,0.5\n"
            f"out,{self.BECH32},1.0\n"
            "exchange,SomeExchangeLabel,2.0\n"
        )
        self.assertEqual(ownership.extract_candidates_from_csv(text), [self.BECH32])

    def test_base58_checksum_harvest(self):
        valid = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"  # genesis P2PKH, valid checksum
        corrupted = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfXX"  # bad checksum
        text = f"address\n{valid}\n{corrupted}\nVacationFund2024\n"
        got = ownership.extract_candidates_from_csv(text)
        self.assertIn(valid, got)
        self.assertNotIn(corrupted, got)  # checksum-invalid base58 rejected
        self.assertNotIn("VacationFund2024", got)  # 8+ char plain word rejected

    def test_unbalanced_quote_recovers_buried_token(self):
        # An unterminated quote swallows later content into one runaway field;
        # the raw-split safety net must still recover the buried txid.
        text = f'address,memo\n{self.BECH32},"unterminated\n{self.TXID},ok\n'
        got = ownership.extract_candidates_from_csv(text)
        self.assertIn(self.BECH32, got)
        self.assertIn(self.TXID, got)

    def test_oversize_field_does_not_crash_and_recovers(self):
        import csv as _csv

        old = _csv.field_size_limit()
        self.addCleanup(_csv.field_size_limit, old)
        _csv.field_size_limit(64)
        text = f"address\n{'a' * 200}\n{self.BECH32}\n"
        self.assertIn(self.BECH32, ownership.extract_candidates_from_csv(text))

    def test_max_harvest_truncation_warns(self):
        cap = ownership.MAX_HARVEST_CANDIDATES
        txids = "\n".join(f"{i:064x}" for i in range(cap + 5))
        report = ownership.identify(_engine_conn(), "p1", csv_text=txids, scan_to_index=0)
        self.assertEqual(len(report["results"]), cap)
        self.assertTrue(report["warnings"])
        self.assertIn("only the first", report["warnings"][0])


class AiCsvStripTests(unittest.TestCase):
    def test_for_ai_builder_drops_csv_text(self):
        # The AI surface must never harvest a CSV: build_wallet_identify_snapshot_for_ai
        # strips csv_text before delegating (defense in depth behind the schema).
        from kassiber.core import ui_snapshot

        captured: dict = {}

        def _fake(conn, runtime_config, args):
            captured["args"] = args
            return {"results": [], "summary": {}, "warnings": []}

        with unittest.mock.patch.object(ui_snapshot, "build_wallet_identify_snapshot", _fake):
            ui_snapshot.build_wallet_identify_snapshot_for_ai(
                None, None, {"csv_text": "bc1qsneaky", "addresses": ["bc1qkeep"]}
            )
        self.assertNotIn("csv_text", captured["args"])
        self.assertEqual(captured["args"].get("addresses"), ["bc1qkeep"])


class IdentifyVerifyTierTests(unittest.TestCase):
    def test_failed_verify_degrades_to_unknown_with_warning(self):
        conn = _engine_conn()

        def raiser(_txid, _chain):
            raise RuntimeError("HTTP 404 Not Found")

        report = ownership.identify(
            conn,
            "p1",
            txids=["ab" * 32],
            scan_to_index=0,
            verify_fetcher=raiser,
        )
        self.assertEqual(report["summary"]["unknown"], 1)
        result = report["results"][0]
        self.assertEqual(result["status"], "unknown")
        self.assertTrue(report["warnings"])
        self.assertIn("On-chain verify", report["warnings"][0])

    def test_successful_verify_classifies_outbound_payment(self):
        conn = _engine_conn()
        # Seed one owned outpoint so the verified tx's input resolves to a wallet.
        conn.execute(
            "INSERT INTO wallet_utxos(profile_id, wallet_id, txid, vout, address, "
            "branch_label, branch_index, address_index, chain, network) "
            "VALUES(?,?,?,?,?,?,?,?,?,?)",
            ("p1", "w1", "aa" * 32, 0, None, "receive", 0, 3, "bitcoin", "main"),
        )
        conn.execute(
            "INSERT INTO wallets(id, profile_id, label, kind, config_json, account_id) "
            "VALUES(?,?,?,?,?,?)",
            ("w1", "p1", "Vault", "descriptor", "{}", None),
        )
        conn.commit()

        def fetcher(_txid, _chain):
            return {
                "inputs": [{"outpoint": "aa" * 32 + ":0", "script": None}],
                "outputs": [{"n": 0, "script": "0014deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"}],
                "chain": "bitcoin",
                "source": "chain",
            }

        report = ownership.identify(
            conn,
            "p1",
            txids=["bb" * 32],
            scan_to_index=0,
            verify_fetcher=fetcher,
        )
        result = report["results"][0]
        self.assertEqual(result["status"], "owned")
        self.assertEqual(result["classification"], "outbound_payment")
        self.assertEqual(result["owned_inputs"], 1)
        self.assertEqual(result["external_outputs"], 1)
        self.assertEqual(result["match_source"], "chain")

    def test_verify_validation_error_reraises(self):
        # A deterministic setup error (e.g. an Electrum backend with no chain)
        # must fail the run, not be silently degraded to a per-txid "unknown".
        from kassiber.errors import AppError

        def bad_backend(_txid, _chain):
            raise AppError("backend needs a chain", code="validation")

        with self.assertRaises(AppError):
            ownership.identify(
                _engine_conn(),
                "p1",
                txids=["ab" * 32],
                scan_to_index=0,
                verify_fetcher=bad_backend,
            )


if __name__ == "__main__":
    unittest.main()
