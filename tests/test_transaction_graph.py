import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import kassiber.core.transaction_graph as tg
from kassiber.backends import DEFAULT_BACKENDS, create_db_backend
from kassiber.core.sync_backends import address_to_scriptpubkey
from kassiber.db import ensure_schema_compat, open_db, set_setting
from kassiber.errors import AppError


NOW = "2026-01-01T00:00:00Z"
BTC = 100_000_000_000
ADDR_A = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
ADDR_B = "bc1q0xcqpzrky6eff2g52qdye53xkk9jxkvrh6yhyw"
ADDR_C = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"
SCRIPT_A = address_to_scriptpubkey(ADDR_A).hex()
SCRIPT_B = address_to_scriptpubkey(ADDR_B).hex()
SCRIPT_C = address_to_scriptpubkey(ADDR_C).hex()


class _FakeElectrumClient:
    calls: list[tuple[str, tuple[str, ...]]] = []
    responses: dict[str, str] = {}
    backends: list[dict] = []

    def __init__(self, backend):
        self.backend = backend
        self.backends.append(dict(backend))

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        return False

    def call(self, method, params=None):
        params = tuple(params or ())
        self.calls.append((method, params))
        txid = str(params[0])
        if txid not in self.responses:
            raise AssertionError(f"Unexpected Electrum tx fetch: {txid}")
        return self.responses[txid]

    def batch_call(self, requests):
        return [self.call(method, params) for method, params in requests]


class _FakeScriptPubKey:
    def __init__(self, data: bytes):
        self.data = data


class _FakeLiquidInput:
    def __init__(self, txid: str, vout: int):
        self.txid = txid
        self.vout = vout


class _FakeLiquidOutput:
    def __init__(self, script_hex: str, *, blinded: bool = True, value: int | None = None):
        self.script_pubkey = _FakeScriptPubKey(bytes.fromhex(script_hex))
        self.is_blinded = blinded
        self.value = value


class _FakeLiquidTx:
    version = 2
    locktime = 0

    def __init__(self, vin, vout):
        self.vin = vin
        self.vout = vout


class TransactionGraphTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-transaction-graph-")
        self.conn = open_db(Path(self.tmp.name) / "data")
        self._seed()

    def tearDown(self):
        self.conn.close()
        self.tmp.cleanup()

    def _seed(self):
        conn = self.conn
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-1", "Main", NOW),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            ("profile-1", "ws-1", "Default", "EUR", "generic", 365, "FIFO", NOW),
        )
        conn.execute(
            """
            INSERT INTO accounts(
                id, workspace_id, profile_id, code, label, account_type, asset, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            ("acct-1", "ws-1", "profile-1", "treasury", "Treasury", "asset", "BTC", NOW),
        )
        for wallet_id, label in (
            ("wallet-a", "Cold"),
            ("wallet-b", "Hot"),
            ("wallet-c", "Vault"),
        ):
            conn.execute(
                """
                INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, config_json, created_at)
                VALUES(?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (wallet_id, "ws-1", "profile-1", "acct-1", label, "custom", "{}", NOW),
            )
        set_setting(conn, "context_workspace", "ws-1")
        set_setting(conn, "context_profile", "profile-1")
        conn.commit()

    def _utxo(
        self,
        wallet_id,
        address,
        txid,
        vout,
        amount=50_000_000,
        *,
        chain="bitcoin",
        network="main",
        asset="BTC",
    ):
        self.conn.execute(
            """
            INSERT INTO wallet_utxos(
                id, workspace_id, profile_id, wallet_id, chain, network, asset,
                amount, txid, vout, outpoint, confirmation_status, address,
                branch_label, branch_index, address_index, first_seen_at, last_seen_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                f"utxo-{wallet_id}-{txid}-{vout}",
                "ws-1",
                "profile-1",
                wallet_id,
                chain,
                network,
                asset,
                amount * 1000,
                txid,
                vout,
                f"{txid}:{vout}",
                "confirmed",
                address,
                "receive",
                0,
                0,
                NOW,
                NOW,
            ),
        )

    def _tx(
        self,
        tx_id,
        wallet_id,
        direction,
        amount_msat,
        external_id,
        raw_json,
        *,
        fee_msat=0,
        asset="BTC",
        kind=None,
        description=None,
        counterparty=None,
    ):
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                fiat_rate, fiat_value, kind, description, counterparty, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                tx_id,
                "ws-1",
                "profile-1",
                wallet_id,
                external_id,
                f"fp-{tx_id}",
                NOW,
                direction,
                asset,
                amount_msat,
                fee_msat,
                "EUR",
                40_000.0,
                None,
                kind or ("withdrawal" if direction == "outbound" else "deposit"),
                description,
                counterparty,
                json.dumps(raw_json, sort_keys=True) if isinstance(raw_json, dict) else raw_json,
                NOW,
            ),
        )

    def _graph(self, transaction, *, allow_public_lookup=False, runtime_config=None):
        self.conn.commit()
        return tg.build_transaction_graph_snapshot(
            self.conn,
            {"transaction": transaction, "allowPublicLookup": allow_public_lookup},
            runtime_config=runtime_config,
        )

    def _cached_graph_raw(self, txid, *, chain="bitcoin", network="main"):
        row = self.conn.execute(
            """
            SELECT payload_json
            FROM transaction_graph_cache
            WHERE schema_version = ? AND chain = ? AND network = ? AND txid = ?
            """,
            (tg.GRAPH_CACHE_SCHEMA_VERSION, chain, network, txid.lower()),
        ).fetchone()
        self.assertIsNotNone(row)
        return json.loads(row["payload_json"])

    def test_transaction_graph_cache_schema_migration_recreates_table(self):
        self.conn.execute("DROP TABLE transaction_graph_cache")
        self.conn.commit()

        ensure_schema_compat(self.conn)

        columns = {
            row["name"]: row
            for row in self.conn.execute("PRAGMA table_info(transaction_graph_cache)")
        }
        self.assertEqual(
            {"schema_version", "chain", "network", "txid", "payload_json", "created_at", "updated_at"},
            set(columns),
        )
        self.assertGreater(columns["schema_version"]["pk"], 0)
        indexes = {
            row["name"]
            for row in self.conn.execute("PRAGMA index_list(transaction_graph_cache)")
        }
        self.assertIn("idx_transaction_graph_cache_updated", indexes)

    def test_esplora_full_graph_returns_curated_model(self):
        self._utxo("wallet-a", ADDR_A, "prevfull", 0, amount=60_000_000)
        self._utxo("wallet-b", ADDR_B, "scan-full-b", 0, amount=50_000_000)
        raw = {
            "txid": "full-tx",
            "version": 2,
            "locktime": 0,
            "size": 300,
            "vsize": 250,
            "weight": 1000,
            "vin": [
                {
                    "txid": "prevfull",
                    "vout": 0,
                    "prevout": {
                        "scriptpubkey": SCRIPT_A,
                        "scriptpubkey_type": "v0_p2wpkh",
                        "scriptpubkey_address": ADDR_A,
                        "value": 60_000_000,
                    },
                }
            ],
            "vout": [
                {"n": 0, "scriptpubkey": SCRIPT_B, "scriptpubkey_address": ADDR_B, "value": 50_000_000},
                {"n": 1, "scriptpubkey": "6a026b62", "scriptpubkey_type": "op_return", "value": 0},
                {"n": 2, "scriptpubkey": "00141111111111111111111111111111111111111111", "value": 9_000_000},
            ],
        }
        self._tx("full-out", "wallet-a", "outbound", 59_000_000_000, "full-tx", raw, fee_msat=1_000_000_000)

        payload = self._graph("full-out")

        self.assertEqual(payload["supportLevel"], "full")
        self.assertEqual(payload["transaction"]["inputCount"], 1)
        self.assertEqual(payload["transaction"]["outputCount"], 3)
        self.assertEqual(payload["fee"]["valueSats"], 1_000_000)
        self.assertEqual(payload["outputs"][0]["role"], "owned_destination")
        self.assertEqual(payload["outputs"][1]["role"], "op_return")
        serialized = json.dumps(payload)
        self.assertNotIn(SCRIPT_A, serialized)
        self.assertNotIn("raw_json", serialized)

    def test_graphless_import_has_clear_state(self):
        self._tx("csv-row", "wallet-a", "outbound", 100_000_000, "csv-1", "{}")

        payload = self._graph("csv-row")

        self.assertEqual(payload["supportLevel"], "graphless")
        self.assertEqual(payload["unsupportedReason"], "graphless_import")
        self.assertEqual(payload["inputs"], [])
        self.assertEqual(payload["outputs"], [])

    def test_decimal_btc_output_values_are_converted_to_sats(self):
        raw = {
            "txid": "decimal-tx",
            "vin": [
                {
                    "txid": "prevdecimal",
                    "vout": 0,
                    "prevout": {"scriptpubkey": SCRIPT_A, "value": 1_000_000},
                }
            ],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 0.005}],
        }
        self._tx("decimal-row", "wallet-a", "outbound", 500_000_000, "decimal-tx", raw)

        payload = self._graph("decimal-row")

        self.assertEqual(payload["outputs"][0]["valueSats"], 500_000)
        self.assertEqual(payload["outputs"][0]["valueBtc"], 0.005)

    def test_raw_hex_derives_public_size_metadata(self):
        raw_hex = (
            "01000000"
            "01"
            f"{'11' * 32}"
            "00000000"
            "00"
            "ffffffff"
            "01"
            "40420f0000000000"
            "00"
            "00000000"
        )
        raw = {
            "txid": "hex-size-tx",
            "raw_hex": raw_hex,
            "vin": [{"txid": "11" * 32, "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 1_000_000}}],
            "vout": [{"n": 0, "scriptpubkey": "", "value": 1_000_000}],
        }
        self._tx("hex-size-row", "wallet-a", "outbound", 1_000_000_000, "hex-size-tx", raw)

        payload = self._graph("hex-size-row")

        self.assertEqual(payload["transaction"]["size"], 60)
        self.assertEqual(payload["transaction"]["vsize"], 60)
        self.assertEqual(payload["transaction"]["weight"], 240)

    def test_bitcoin_missing_input_prevout_values_are_explained_precisely(self):
        raw = {
            "txid": "prevout-missing-tx",
            "vin": [{"txid": "22" * 32, "vout": 0}],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 500_000}],
        }
        self._tx(
            "prevout-missing-row",
            "wallet-a",
            "inbound",
            500_000_000,
            "prevout-missing-tx",
            raw,
        )

        payload = self._graph("prevout-missing-row")

        self.assertEqual(payload["supportLevel"], "partial")
        self.assertEqual(payload["unsupportedReason"], "input_prevout_values_missing")
        self.assertEqual(payload["outputs"][0]["valueSats"], 500_000)
        self.assertIn("spent previous output", payload["warnings"][0]["message"])

    def test_inbound_owned_output_is_not_marked_change(self):
        self._utxo("wallet-b", ADDR_B, "receive-tx", 0, amount=9_210)
        raw = {
            "txid": "receive-tx",
            "vin": [
                {
                    "txid": "55" * 32,
                    "vout": 0,
                    "prevout": {
                        "scriptpubkey": "0014" + "88" * 20,
                        "value": 30_000,
                    },
                }
            ],
            "vout": [
                {"n": 0, "scriptpubkey": SCRIPT_B, "scriptpubkey_address": ADDR_B, "value": 9_210},
                {"n": 1, "scriptpubkey": "0014" + "99" * 20, "value": 18_654},
            ],
        }
        self._tx("receive-row", "wallet-b", "inbound", 9_210_000, "receive-tx", raw)

        payload = self._graph("receive-row")

        self.assertEqual(payload["outputs"][0]["role"], "incoming_payment")
        self.assertNotIn("change", {annotation["code"] for annotation in payload["outputs"][0]["annotations"]})
        self.assertEqual(payload["outputs"][1]["role"], "external_recipient")

    def test_inbound_graph_infers_payment_output_from_recorded_amount(self):
        txid = "82" * 32
        raw = {
            "txid": txid,
            "vin": [
                {
                    "txid": "55" * 32,
                    "vout": 0,
                    "prevout": {
                        "scriptpubkey": "0014" + "88" * 20,
                        "value": 312_414_797,
                    },
                }
            ],
            "vout": [
                {
                    "n": 0,
                    "scriptpubkey": SCRIPT_B,
                    "scriptpubkey_address": ADDR_B,
                    "value": 26_705,
                },
                {"n": 1, "scriptpubkey": "0014" + "99" * 20, "value": 312_387_078},
            ],
        }
        self._tx(
            "inbound-amount-row",
            "wallet-b",
            "inbound",
            26_705_000,
            txid,
            raw,
        )

        payload = self._graph("inbound-amount-row")

        self.assertEqual(payload["outputs"][0]["role"], "incoming_payment")
        self.assertEqual(payload["outputs"][0]["ownership"], "owned")
        self.assertIn(
            "recorded_incoming_amount",
            {annotation["code"] for annotation in payload["outputs"][0]["annotations"]},
        )
        self.assertEqual(payload["outputs"][1]["role"], "external_recipient")

    def test_inbound_amount_inference_skips_ambiguous_equal_outputs(self):
        txid = "83" * 32
        raw = {
            "txid": txid,
            "vin": [
                {
                    "txid": "56" * 32,
                    "vout": 0,
                    "prevout": {
                        "scriptpubkey": "0014" + "88" * 20,
                        "value": 20_500,
                    },
                }
            ],
            "vout": [
                {"n": 0, "scriptpubkey": SCRIPT_B, "value": 10_000},
                {"n": 1, "scriptpubkey": "0014" + "99" * 20, "value": 10_000},
            ],
        }
        self._tx(
            "ambiguous-inbound-amount-row",
            "wallet-b",
            "inbound",
            10_000_000,
            txid,
            raw,
        )

        payload = self._graph("ambiguous-inbound-amount-row")

        self.assertEqual(
            [output["role"] for output in payload["outputs"]],
            ["external_recipient", "external_recipient"],
        )

    def test_inbound_amount_inference_skips_when_owned_output_matched(self):
        # The receiving wallet already covers ADDR_B, so the real incoming
        # output is matched through the normal owned-index path. A *different*
        # external output that merely equals the recorded amount must not be
        # relabelled as an owned incoming payment.
        self._utxo("wallet-b", ADDR_B, "coincidence-tx", 0, amount=30_000)
        txid = "84" * 32
        raw = {
            "txid": txid,
            "vin": [
                {
                    "txid": "57" * 32,
                    "vout": 0,
                    "prevout": {
                        "scriptpubkey": "0014" + "88" * 20,
                        "value": 312_414_797,
                    },
                }
            ],
            "vout": [
                {
                    "n": 0,
                    "scriptpubkey": SCRIPT_B,
                    "scriptpubkey_address": ADDR_B,
                    "value": 30_000,
                },
                {"n": 1, "scriptpubkey": "0014" + "99" * 20, "value": 26_705},
            ],
        }
        self._tx(
            "coincidental-inbound-amount-row",
            "wallet-b",
            "inbound",
            26_705_000,
            txid,
            raw,
        )

        payload = self._graph("coincidental-inbound-amount-row")

        # Owned output is the genuine receive; the coincidental external output
        # stays external and is never inferred as owned.
        self.assertEqual(payload["outputs"][0]["role"], "incoming_payment")
        self.assertEqual(payload["outputs"][0]["ownership"], "owned")
        self.assertEqual(payload["outputs"][1]["role"], "external_recipient")
        self.assertEqual(payload["outputs"][1]["ownership"], "external")

    def test_bitcoin_missing_prevout_values_are_enriched_from_public_lookup(self):
        txid = "33" * 32
        raw = {
            "txid": txid,
            "vin": [{"txid": "44" * 32, "vout": 0}],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 500_000}],
        }
        fetched = {
            "txid": txid,
            "version": 2,
            "locktime": 0,
            "vsize": 141,
            "vin": [
                {
                    "txid": "44" * 32,
                    "vout": 0,
                    "prevout": {
                        "scriptpubkey": SCRIPT_A,
                        "scriptpubkey_type": "v0_p2wpkh",
                        "scriptpubkey_address": ADDR_A,
                        "value": 600_000,
                    },
                }
            ],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 500_000}],
        }
        create_db_backend(
            self.conn,
            "graph-mempool",
            "mempool",
            "https://mempool.example/api",
            chain="bitcoin",
            network="main",
            timeout=5,
            commit=False,
        )
        self._tx("prevout-enriched-row", "wallet-a", "inbound", 500_000_000, txid, raw)

        with patch(
            "kassiber.core.transaction_graph.fetch_esplora_transaction",
            return_value=fetched,
        ) as fetch:
            payload = self._graph("prevout-enriched-row", allow_public_lookup=True)

        fetch.assert_called_once_with("https://mempool.example/api", txid, timeout=5)
        self.assertEqual(payload["supportLevel"], "full")
        self.assertIsNone(payload["unsupportedReason"])
        self.assertEqual(payload["inputs"][0]["valueSats"], 600_000)
        self.assertEqual(payload["fee"]["valueSats"], 100_000)
        self.assertEqual(payload["fee"]["rateSatVb"], 709.22)
        self.assertNotIn(
            "input_prevout_values_missing",
            {warning["code"] for warning in payload["warnings"]},
        )

    def test_bitcoin_graphless_txid_row_fetches_public_graph(self):
        txid = "88" * 32
        fetched = {
            "txid": txid,
            "version": 2,
            "locktime": 0,
            "vsize": 141,
            "vin": [
                {
                    "txid": "99" * 32,
                    "vout": 1,
                    "prevout": {
                        "scriptpubkey": SCRIPT_A,
                        "scriptpubkey_type": "v0_p2wpkh",
                        "scriptpubkey_address": ADDR_A,
                        "value": 1_000_000,
                    },
                }
            ],
            "vout": [
                {"n": 0, "scriptpubkey": "0014" + "11" * 20, "value": 800_000},
                {"n": 1, "scriptpubkey": "0014" + "22" * 20, "value": 198_000},
            ],
        }
        create_db_backend(
            self.conn,
            "graph-mempool",
            "mempool",
            "https://mempool.example/api",
            chain="bitcoin",
            network="main",
            timeout=5,
            commit=False,
        )
        self._tx("bull-row", "wallet-a", "outbound", 800_000_000, txid, "{}", fee_msat=2_000_000)

        with patch(
            "kassiber.core.transaction_graph.fetch_esplora_transaction",
            return_value=fetched,
        ) as fetch:
            payload = self._graph("bull-row", allow_public_lookup=True)

        fetch.assert_called_once_with("https://mempool.example/api", txid, timeout=5)
        self.assertEqual(payload["supportLevel"], "full")
        self.assertIsNone(payload["unsupportedReason"])
        self.assertEqual(payload["transaction"]["inputCount"], 1)
        self.assertEqual(payload["transaction"]["outputCount"], 2)
        self.assertEqual(payload["inputs"][0]["outpoint"], f"{'99' * 32}:1")
        self.assertEqual(payload["inputs"][0]["valueSats"], 1_000_000)
        self.assertEqual(payload["outputs"][0]["valueSats"], 800_000)
        self.assertEqual(payload["fee"]["valueSats"], 2_000)
        self.assertNotIn(
            "graphless_import",
            {warning["code"] for warning in payload["warnings"]},
        )

    def test_bitcoin_graphless_txid_row_fetches_bitcoin_core_graph(self):
        txid = "8a" * 32
        prev_txid = "9b" * 32
        create_db_backend(
            self.conn,
            "core-regtest",
            "bitcoinrpc",
            "http://127.0.0.1:18443",
            chain="bitcoin",
            network="regtest",
            config={"username": "kassiber", "password": "secret"},
            commit=False,
        )
        self.conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = ?",
            (json.dumps({"chain": "bitcoin", "network": "regtest"}), "wallet-a"),
        )
        self._tx(
            "core-row",
            "wallet-a",
            "outbound",
            800_000_000,
            txid,
            "{}",
            fee_msat=2_000_000,
        )
        decoded = {
            "txid": txid,
            "version": 2,
            "locktime": 0,
            "size": 141,
            "vsize": 141,
            "weight": 564,
            "vin": [{"txid": prev_txid, "vout": 1, "sequence": 0xFFFFFFFF}],
            "vout": [
                {
                    "n": 0,
                    "value": 0.008,
                    "scriptPubKey": {
                        "hex": "0014" + "11" * 20,
                        "type": "witness_v0_keyhash",
                        "address": ADDR_B,
                    },
                },
                {
                    "n": 1,
                    "value": 0.00198,
                    "scriptPubKey": {"hex": "0014" + "22" * 20, "type": "witness_v0_keyhash"},
                },
            ],
        }
        previous = {
            "txid": prev_txid,
            "vout": [
                {"n": 0, "value": 0.001, "scriptPubKey": {"hex": SCRIPT_A}},
                {
                    "n": 1,
                    "value": 0.01,
                    "scriptPubKey": {
                        "hex": SCRIPT_A,
                        "type": "witness_v0_keyhash",
                        "address": ADDR_A,
                    },
                },
            ],
        }

        def fake_rpc(_backend, method, params=None, **_kwargs):
            self.assertEqual(method, "getrawtransaction")
            lookup_txid = params[0]
            if lookup_txid == txid:
                return decoded
            if lookup_txid == prev_txid:
                return previous
            raise AssertionError(f"unexpected txid {lookup_txid}")

        with patch("kassiber.core.transaction_graph.bitcoinrpc_call", side_effect=fake_rpc) as rpc:
            payload = self._graph("core-row", allow_public_lookup=True)

        self.assertEqual(rpc.call_count, 2)
        self.assertEqual(payload["supportLevel"], "full")
        self.assertIsNone(payload["unsupportedReason"])
        self.assertEqual(payload["transaction"]["vsize"], 141)
        self.assertEqual(payload["inputs"][0]["outpoint"], f"{prev_txid}:1")
        self.assertEqual(payload["inputs"][0]["valueSats"], 1_000_000)
        self.assertEqual(payload["outputs"][0]["address"], ADDR_B)
        self.assertEqual(payload["outputs"][0]["valueSats"], 800_000)
        self.assertEqual(payload["fee"]["valueSats"], 2_000)

    def test_bitcoin_graph_lookup_falls_back_from_electrum_to_core_for_coinbase(self):
        txid = "8b" * 32
        create_db_backend(
            self.conn,
            "graph-fulcrum",
            "electrum",
            "tcp://127.0.0.1:18548",
            chain="bitcoin",
            network="regtest",
            timeout=5,
            commit=False,
        )
        create_db_backend(
            self.conn,
            "core-regtest",
            "bitcoinrpc",
            "http://127.0.0.1:18443",
            chain="bitcoin",
            network="regtest",
            config={"username": "kassiber", "password": "secret"},
            commit=False,
        )
        self.conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = ?",
            (json.dumps({"chain": "bitcoin", "network": "regtest"}), "wallet-a"),
        )
        self._tx(
            "mining-row",
            "wallet-a",
            "inbound",
            1_250_000_000_000,
            txid,
            "{}",
            description="Regtest Mining Rig",
        )
        decoded = {
            "txid": txid,
            "version": 2,
            "locktime": 0,
            "vin": [{"coinbase": "04ffff001d0104", "sequence": 0xFFFFFFFF}],
            "vout": [
                {
                    "n": 0,
                    "value": 12.5,
                    "scriptPubKey": {
                        "hex": SCRIPT_A,
                        "type": "witness_v0_keyhash",
                        "address": ADDR_A,
                    },
                },
            ],
        }

        def fake_rpc(_backend, method, params=None, **_kwargs):
            self.assertEqual(method, "getrawtransaction")
            self.assertEqual(params[0], txid)
            return decoded

        _FakeElectrumClient.calls = []
        _FakeElectrumClient.responses = {}
        with patch("kassiber.core.transaction_graph.ElectrumClient", _FakeElectrumClient), patch(
            "kassiber.core.transaction_graph.bitcoinrpc_call",
            side_effect=fake_rpc,
        ) as rpc:
            payload = self._graph("mining-row", allow_public_lookup=True)

        self.assertEqual(
            _FakeElectrumClient.calls,
            [("blockchain.transaction.get", (txid,))],
        )
        self.assertEqual(rpc.call_count, 1)
        self.assertEqual(payload["supportLevel"], "partial")
        self.assertEqual(payload["unsupportedReason"], "input_prevout_values_missing")
        self.assertEqual(payload["transaction"]["outputCount"], 1)
        self.assertEqual(payload["outputs"][0]["valueSats"], 1_250_000_000)
        self.assertNotIn(
            "graphless_import",
            {warning["code"] for warning in payload["warnings"]},
        )

    def test_graph_lookup_uses_silent_payment_backend_forwarding(self):
        txid = "31" * 32
        runtime_config = {
            "default_backend": "bitcoin-frigate-regtest",
            "backends": {
                "bitcoin-frigate-regtest": {
                    "kind": "electrum",
                    "chain": "bitcoin",
                    "network": "regtest",
                    "url": "tcp://127.0.0.1:18548",
                    "silent_payments": True,
                    "silent_payment_scan_file": "/tmp/kassiber-sp-scan.json",
                }
            },
        }
        self.conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = ?",
            (json.dumps({"chain": "bitcoin", "network": "regtest"}), "wallet-a"),
        )
        self._tx(
            "silent-payment-row",
            "wallet-a",
            "inbound",
            1_234_567_000,
            txid,
            {"outputs": [{"amount_sats": 1_234_567, "silent_payment": True}]},
            description="Silent Payment sync from bitcoin-frigate-regtest",
        )
        decoded = {
            "version": 2,
            "locktime": 0,
            "vin": [],
            "vout": [
                {
                    "n": 0,
                    "value": 0.01234567,
                    "scriptPubKey": {
                        "hex": "5120" + "11" * 32,
                        "type": "witness_v1_taproot",
                    },
                }
            ],
        }
        _FakeElectrumClient.calls = []
        _FakeElectrumClient.backends = []
        _FakeElectrumClient.responses = {txid: "current-raw"}

        with patch("kassiber.core.transaction_graph.ElectrumClient", _FakeElectrumClient), patch(
            "kassiber.core.transaction_graph.decode_raw_transaction",
            return_value=decoded,
        ):
            payload = self._graph(
                "silent-payment-row",
                allow_public_lookup=True,
                runtime_config=runtime_config,
            )

        self.assertEqual(
            _FakeElectrumClient.calls,
            [("blockchain.transaction.get", (txid,))],
        )
        self.assertEqual(_FakeElectrumClient.backends[0]["name"], "bitcoin-frigate-regtest")
        self.assertEqual(payload["supportLevel"], "full")
        self.assertEqual(payload["transaction"]["inputCount"], 0)
        self.assertEqual(payload["transaction"]["outputCount"], 1)

    def test_bitcoin_core_graph_dedups_shared_prevout_txid(self):
        # Two inputs spending different vouts of the SAME previous txid must
        # resolve with a single deduplicated getrawtransaction for that prev tx
        # (2 RPCs total: the focused tx + one shared prevout), not one per input.
        txid = "8c" * 32
        prev_txid = "9d" * 32
        create_db_backend(
            self.conn,
            "core-regtest",
            "bitcoinrpc",
            "http://127.0.0.1:18443",
            chain="bitcoin",
            network="regtest",
            config={"username": "kassiber", "password": "secret"},
            commit=False,
        )
        self.conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = ?",
            (json.dumps({"chain": "bitcoin", "network": "regtest"}), "wallet-a"),
        )
        self._tx(
            "core-dedup-row",
            "wallet-a",
            "outbound",
            1_500_000_000,
            txid,
            "{}",
            fee_msat=1_000_000,
        )
        decoded = {
            "txid": txid,
            "version": 2,
            "locktime": 0,
            "vsize": 200,
            "vin": [
                {"txid": prev_txid, "vout": 0, "sequence": 0xFFFFFFFF},
                {"txid": prev_txid, "vout": 1, "sequence": 0xFFFFFFFF},
            ],
            "vout": [
                {
                    "n": 0,
                    "value": 0.015,
                    "scriptPubKey": {
                        "hex": "0014" + "11" * 20,
                        "type": "witness_v0_keyhash",
                        "address": ADDR_B,
                    },
                },
            ],
        }
        previous = {
            "txid": prev_txid,
            "vout": [
                {
                    "n": 0,
                    "value": 0.006,
                    "scriptPubKey": {
                        "hex": SCRIPT_A,
                        "type": "witness_v0_keyhash",
                        "address": ADDR_A,
                    },
                },
                {
                    "n": 1,
                    "value": 0.01,
                    "scriptPubKey": {
                        "hex": SCRIPT_A,
                        "type": "witness_v0_keyhash",
                        "address": ADDR_A,
                    },
                },
            ],
        }
        prev_lookups = 0

        def fake_rpc(_backend, method, params=None, **_kwargs):
            nonlocal prev_lookups
            self.assertEqual(method, "getrawtransaction")
            lookup_txid = params[0]
            if lookup_txid == txid:
                return decoded
            if lookup_txid == prev_txid:
                prev_lookups += 1
                return previous
            raise AssertionError(f"unexpected txid {lookup_txid}")

        with patch("kassiber.core.transaction_graph.bitcoinrpc_call", side_effect=fake_rpc) as rpc:
            payload = self._graph("core-dedup-row", allow_public_lookup=True)

        # Shared prev txid fetched exactly once despite two inputs referencing it.
        self.assertEqual(prev_lookups, 1)
        self.assertEqual(rpc.call_count, 2)
        self.assertEqual(payload["supportLevel"], "full")
        self.assertEqual(payload["inputs"][0]["outpoint"], f"{prev_txid}:0")
        self.assertEqual(payload["inputs"][0]["valueSats"], 600_000)
        self.assertEqual(payload["inputs"][1]["outpoint"], f"{prev_txid}:1")
        self.assertEqual(payload["inputs"][1]["valueSats"], 1_000_000)

    def test_public_graph_lookup_populates_cache_and_reuses_it(self):
        txid = "8a" * 32
        backend_url = "https://mempool.example/api?token=do-not-cache"
        fetched = {
            "txid": txid,
            "version": 2,
            "locktime": 0,
            "vsize": 141,
            "raw_hex": "00" * 80,
            "hex": "11" * 80,
            "backend_url": backend_url,
            "descriptor": "wpkh([fingerprint/84h]xpubSECRET/0/*)",
            "xpub": "xpubSECRET",
            "token": "secret-token",
            "raw_config": {"url": backend_url, "token": "secret-token"},
            "raw_daemon_args": ["--token", "secret-token"],
            "source_file": "/tmp/import.csv",
            "vin": [
                {
                    "txid": "8b" * 32,
                    "vout": 1,
                    "prevout": {
                        "scriptpubkey": SCRIPT_A,
                        "scriptpubkey_type": "v0_p2wpkh",
                        "scriptpubkey_address": ADDR_A,
                        "value": 1_000_000,
                    },
                }
            ],
            "vout": [
                {"n": 0, "scriptpubkey": SCRIPT_B, "value": 900_000},
            ],
        }
        create_db_backend(
            self.conn,
            "graph-mempool",
            "mempool",
            backend_url,
            chain="bitcoin",
            network="main",
            timeout=5,
            commit=False,
        )
        self._tx("cache-http-row", "wallet-a", "outbound", 900_000_000, txid, "{}")

        with patch(
            "kassiber.core.transaction_graph.fetch_esplora_transaction",
            return_value=fetched,
        ) as fetch:
            first = self._graph("cache-http-row", allow_public_lookup=True)

        fetch.assert_called_once_with("https://mempool.example/api", txid, timeout=5)
        self.assertEqual(first["supportLevel"], "full")
        cached = self._cached_graph_raw(txid)
        cached_serialized = json.dumps(cached, sort_keys=True)
        self.assertEqual(cached["txid"], txid)
        self.assertNotIn("raw_hex", cached_serialized)
        self.assertNotIn('"hex"', cached_serialized)
        self.assertNotIn("backend_url", cached_serialized)
        self.assertNotIn("do-not-cache", cached_serialized)
        self.assertNotIn("xpubSECRET", cached_serialized)
        self.assertNotIn("secret-token", cached_serialized)
        self.assertNotIn("raw_config", cached_serialized)
        self.assertNotIn("raw_daemon_args", cached_serialized)
        self.assertNotIn("source_file", cached_serialized)

        with patch("kassiber.core.transaction_graph.fetch_esplora_transaction") as fetch:
            second = self._graph("cache-http-row", allow_public_lookup=True)

        fetch.assert_not_called()
        self.assertEqual(second["supportLevel"], "full")
        self.assertIsNone(second["unsupportedReason"])
        self.assertEqual(second["transaction"]["inputCount"], 1)
        self.assertEqual(second["transaction"]["outputCount"], 1)
        self.assertEqual(second["inputs"][0]["valueSats"], 1_000_000)
        self.assertEqual(second["fee"]["valueSats"], 100_000)

    def test_liquid_confidential_shape_is_reference_only(self):
        raw = {
            "txid": "liquid-tx",
            "vin": [{"txid": "liquid-prev", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}}],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "valuecommitment": "09" + "aa" * 32}],
        }
        self._tx("liquid-row", "wallet-a", "outbound", 1_000_000, "liquid-tx", raw, asset="LBTC")

        payload = self._graph("liquid-row")

        self.assertEqual(payload["supportLevel"], "partial")
        self.assertEqual(payload["unsupportedReason"], "confidential_values_hidden")
        self.assertEqual(payload["outputs"][0]["valueState"], "confidential")
        self.assertNotIn("valueSats", payload["outputs"][0])

    def test_liquid_graphless_row_fetches_amountless_reference_graph(self):
        txid = "66" * 32
        fetched = {
            "txid": txid,
            "version": 2,
            "locktime": 0,
            "vin": [
                {
                    "txid": "77" * 32,
                    "vout": 0,
                    "prevout": {
                        "scriptpubkey": SCRIPT_A,
                        "valuecommitment": "09" + "aa" * 32,
                        "assetcommitment": "0a" + "bb" * 32,
                    },
                }
            ],
            "vout": [
                {
                    "n": 0,
                    "scriptpubkey": SCRIPT_B,
                    "valuecommitment": "09" + "cc" * 32,
                    "assetcommitment": "0a" + "dd" * 32,
                },
                {
                    "n": 1,
                    "scriptpubkey": SCRIPT_C,
                    "valuecommitment": "09" + "ee" * 32,
                    "assetcommitment": "0a" + "ff" * 32,
                },
            ],
        }
        create_db_backend(
            self.conn,
            "graph-liquid",
            "mempool",
            "https://liquid.example/api",
            chain="liquid",
            network="liquidv1",
            timeout=5,
            commit=False,
        )
        self._tx("liquid-reference-row", "wallet-a", "inbound", 25_022_000, txid, "{}", asset="LBTC")

        with patch(
            "kassiber.core.transaction_graph.fetch_esplora_transaction",
            return_value=fetched,
        ) as fetch:
            payload = self._graph("liquid-reference-row", allow_public_lookup=True)

        # Fetched only from the configured Liquid backend — never a hardcoded host.
        fetch.assert_called_once_with("https://liquid.example/api", txid, timeout=5)
        self.assertEqual(payload["supportLevel"], "partial")
        self.assertEqual(payload["unsupportedReason"], "confidential_values_hidden")
        self.assertEqual(payload["transaction"]["inputCount"], 1)
        self.assertEqual(payload["transaction"]["outputCount"], 2)
        self.assertEqual(payload["inputs"][0]["outpoint"], f"{'77' * 32}:0")
        self.assertEqual(payload["inputs"][0]["valueState"], "confidential")
        self.assertEqual(payload["outputs"][0]["outpoint"], f"{txid}:0")
        self.assertEqual(payload["outputs"][0]["valueState"], "confidential")
        self.assertNotIn("valueSats", payload["inputs"][0])
        self.assertNotIn("valueSats", payload["outputs"][0])
        serialized = json.dumps(payload)
        self.assertNotIn("valuecommitment", serialized)
        self.assertNotIn("assetcommitment", serialized)

    def test_liquid_reference_graph_uses_local_wallet_amounts(self):
        txid = "65" * 32
        prev_txid = "64" * 32
        self._utxo(
            "wallet-a",
            ADDR_A,
            prev_txid,
            0,
            amount=13_000_000,
            chain="liquid",
            network="liquidv1",
            asset="LBTC",
        )
        self._utxo(
            "wallet-b",
            ADDR_B,
            txid,
            0,
            amount=12_900_000,
            chain="liquid",
            network="liquidv1",
            asset="LBTC",
        )
        fetched = {
            "txid": txid,
            "version": 2,
            "locktime": 0,
            "vin": [
                {
                    "txid": prev_txid,
                    "vout": 0,
                    "prevout": {
                        "scriptpubkey": SCRIPT_A,
                        "valuecommitment": "09" + "aa" * 32,
                    },
                }
            ],
            "vout": [
                {
                    "n": 0,
                    "scriptpubkey": SCRIPT_B,
                    "valuecommitment": "09" + "bb" * 32,
                },
                {"n": 1, "scriptpubkey_type": "fee", "value": 100_000},
            ],
        }
        create_db_backend(
            self.conn,
            "graph-liquid-local-values",
            "mempool",
            "https://liquid.example/api",
            chain="liquid",
            network="liquidv1",
            timeout=5,
            commit=False,
        )
        self._tx(
            "liquid-local-values-row",
            "wallet-a",
            "outbound",
            12_900_000_000,
            txid,
            "{}",
            asset="LBTC",
        )

        with patch(
            "kassiber.core.transaction_graph.fetch_esplora_transaction",
            return_value=fetched,
        ):
            payload = self._graph("liquid-local-values-row", allow_public_lookup=True)

        self.assertEqual(payload["supportLevel"], "full")
        self.assertIsNone(payload["unsupportedReason"])
        self.assertEqual(payload["inputs"][0]["valueState"], "known")
        self.assertEqual(payload["inputs"][0]["valueSats"], 13_000_000)
        self.assertEqual(payload["outputs"][0]["valueState"], "known")
        self.assertEqual(payload["outputs"][0]["valueSats"], 12_900_000)
        self.assertEqual(payload["fee"]["valueSats"], 100_000)
        self.assertNotIn(
            "confidential_values_hidden",
            {warning["code"] for warning in payload["warnings"]},
        )
        cached = self._cached_graph_raw(txid, chain="liquid", network="liquidv1")
        self.assertEqual(cached["vin"][0]["prevout"]["value_state"], "confidential")
        self.assertEqual(cached["vout"][0]["value_state"], "confidential")

    def test_liquid_runtime_http_backend_is_used(self):
        txid = "6b" * 32
        fetched = {
            "txid": txid,
            "version": 2,
            "locktime": 0,
            "vin": [
                {
                    "txid": "6c" * 32,
                    "vout": 0,
                    "prevout": {
                        "scriptpubkey": SCRIPT_A,
                        "valuecommitment": "09" + "aa" * 32,
                    },
                }
            ],
            "vout": [
                {
                    "n": 0,
                    "scriptpubkey": SCRIPT_B,
                    "valuecommitment": "09" + "bb" * 32,
                }
            ],
        }
        runtime_config = {
            "default_backend": "liquid-http",
            "backends": {
                "liquid-http": {
                    "kind": "liquid-esplora",
                    "chain": "liquid",
                    "network": "liquidv1",
                    "url": "https://runtime-liquid.example/api",
                    "timeout": 60,
                }
            },
        }
        self._tx("liquid-runtime-row", "wallet-a", "inbound", 25_022_000, txid, "{}", asset="LBTC")

        with patch(
            "kassiber.core.transaction_graph.fetch_esplora_transaction",
            return_value=fetched,
        ) as fetch:
            payload = self._graph(
                "liquid-runtime-row",
                allow_public_lookup=True,
                runtime_config=runtime_config,
            )

        fetch.assert_called_once_with("https://runtime-liquid.example/api", txid, timeout=5)
        self.assertEqual(payload["supportLevel"], "partial")
        self.assertEqual(payload["unsupportedReason"], "confidential_values_hidden")
        self.assertEqual(payload["inputs"][0]["valueState"], "confidential")
        self.assertEqual(payload["outputs"][0]["valueState"], "confidential")
        cached_serialized = json.dumps(self._cached_graph_raw(txid, chain="liquid", network="liquidv1"))
        self.assertNotIn("runtime-liquid.example", cached_serialized)
        self.assertNotIn("valuecommitment", cached_serialized)

    def test_liquid_lookup_does_not_accept_chainless_bitcoin_backend(self):
        txid = "6d" * 32
        create_db_backend(
            self.conn,
            "legacy-bitcoin-mempool",
            "mempool",
            "https://bitcoin.example/api",
            timeout=5,
            commit=False,
        )
        self._tx("liquid-chainless-row", "wallet-a", "inbound", 11_000_000, txid, "{}", asset="LBTC")

        with patch("kassiber.core.transaction_graph.fetch_esplora_transaction") as fetch:
            payload = self._graph("liquid-chainless-row", allow_public_lookup=True)

        fetch.assert_not_called()
        self.assertEqual(payload["supportLevel"], "graphless")
        self.assertEqual(payload["unsupportedReason"], "liquid_reference_graph_not_local")
        warning_codes = {warning["code"] for warning in payload["warnings"]}
        self.assertIn("liquid_reference_lookup_unavailable", warning_codes)

    def test_liquid_electrum_cached_confidential_graph_remains_amountless(self):
        txid = "69" * 32
        prev_txid = "6a" * 32
        create_db_backend(
            self.conn,
            "liquid-fulcrum",
            "electrum",
            "ssl://liquid.example:995",
            chain="liquid",
            network="liquidv1",
            timeout=60,
            commit=False,
        )
        self._tx("liquid-electrum-row", "wallet-a", "inbound", 25_022_000, txid, "{}", asset="LBTC")
        decoded = _FakeLiquidTx(
            [_FakeLiquidInput(prev_txid, 0)],
            [
                _FakeLiquidOutput(SCRIPT_B, blinded=True),
                _FakeLiquidOutput("", blinded=False, value=250),
            ],
        )
        _FakeElectrumClient.calls = []
        _FakeElectrumClient.backends = []
        _FakeElectrumClient.responses = {txid: "liquid-current-raw"}

        with patch("kassiber.core.transaction_graph.ElectrumClient", _FakeElectrumClient), patch(
            "kassiber.core.transaction_graph.decode_liquid_transaction",
            return_value=decoded,
        ) as decode:
            first = self._graph("liquid-electrum-row", allow_public_lookup=True)

        self.assertEqual(
            _FakeElectrumClient.calls,
            [("blockchain.transaction.get", (txid,))],
        )
        self.assertEqual(_FakeElectrumClient.backends[0]["timeout"], 5)
        decode.assert_called_once_with("liquid-current-raw")
        self.assertEqual(first["supportLevel"], "partial")
        self.assertEqual(first["unsupportedReason"], "confidential_values_hidden")
        self.assertEqual(first["inputs"][0]["valueState"], "confidential")
        self.assertEqual(first["outputs"][0]["valueState"], "confidential")
        self.assertNotIn("valueSats", first["inputs"][0])
        self.assertNotIn("valueSats", first["outputs"][0])
        self.assertEqual(first["fee"]["valueSats"], 250)

        cached = self._cached_graph_raw(txid, chain="liquid", network="liquidv1")
        cached_serialized = json.dumps(cached, sort_keys=True)
        self.assertEqual(cached["vin"][0]["prevout"]["value_state"], "confidential")
        self.assertEqual(cached["vout"][0]["value_state"], "confidential")
        self.assertNotIn("liquid-current-raw", cached_serialized)
        self.assertNotIn("valuecommitment", cached_serialized)
        self.assertNotIn("assetcommitment", cached_serialized)

        _FakeElectrumClient.calls = []
        _FakeElectrumClient.responses = {}
        with patch("kassiber.core.transaction_graph.ElectrumClient", _FakeElectrumClient), patch(
            "kassiber.core.transaction_graph.decode_liquid_transaction"
        ) as decode:
            second = self._graph("liquid-electrum-row", allow_public_lookup=True)

        self.assertEqual(_FakeElectrumClient.calls, [])
        decode.assert_not_called()
        self.assertEqual(second["outputs"][0]["valueState"], "confidential")
        self.assertEqual(second["fee"]["valueSats"], 250)

    def test_liquid_lookup_without_configured_backend_does_not_fetch(self):
        # Symmetry with Bitcoin: with no configured Liquid explorer, the lookup is
        # declined (no silent third-party fetch) and a warning is surfaced.
        txid = "68" * 32
        self._tx("liquid-no-backend", "wallet-a", "inbound", 11_000_000, txid, "{}", asset="LBTC")

        with patch("kassiber.core.transaction_graph.fetch_esplora_transaction") as fetch:
            payload = self._graph("liquid-no-backend", allow_public_lookup=True)

        fetch.assert_not_called()
        self.assertEqual(payload["supportLevel"], "graphless")
        self.assertEqual(payload["unsupportedReason"], "liquid_reference_graph_not_local")
        self.assertIn("Add a Liquid explorer backend", payload["warnings"][0]["message"])
        self.assertNotIn("Liquid Network", payload["warnings"][0]["message"])
        warning_codes = {warning["code"] for warning in payload["warnings"]}
        self.assertIn("liquid_reference_lookup_unavailable", warning_codes)

    def test_liquid_lookup_falls_back_to_next_configured_backend(self):
        txid = "6a" * 32
        create_db_backend(
            self.conn,
            "liquid-down",
            "mempool",
            "https://down-liquid.example/api",
            chain="liquid",
            network="liquidv1",
            timeout=5,
            commit=False,
        )
        create_db_backend(
            self.conn,
            "liquid-up",
            "mempool",
            "https://up-liquid.example/api",
            chain="liquid",
            network="liquidv1",
            timeout=5,
            commit=False,
        )
        set_setting(self.conn, "default_backend", "liquid-down")
        fetched = {
            "txid": txid,
            "vin": [{"txid": "6b" * 32, "vout": 0}],
            "vout": [
                {"n": 0, "scriptpubkey": SCRIPT_B},
                {"n": 1, "scriptpubkey_type": "fee", "value": 19},
            ],
        }

        def fake_fetch(url, txid_arg, **kwargs):
            if url == "https://down-liquid.example/api":
                raise RuntimeError("backend unavailable")
            self.assertEqual(url, "https://up-liquid.example/api")
            self.assertEqual(txid_arg, txid)
            self.assertEqual(kwargs, {"timeout": 5})
            return fetched

        self._tx("liquid-fallback-row", "wallet-a", "inbound", 11_000_000, txid, "{}", asset="LBTC")

        with patch("kassiber.core.transaction_graph.fetch_esplora_transaction", side_effect=fake_fetch) as fetch:
            payload = self._graph("liquid-fallback-row", allow_public_lookup=True)

        self.assertEqual(
            [call.args[0] for call in fetch.call_args_list],
            ["https://down-liquid.example/api", "https://up-liquid.example/api"],
        )
        self.assertEqual(payload["supportLevel"], "partial")
        self.assertEqual(payload["unsupportedReason"], "confidential_values_hidden")
        self.assertEqual(payload["fee"]["valueSats"], 19)

    def test_public_lookup_is_not_automatic_for_graphless_rows(self):
        txid = "12" * 32
        create_db_backend(
            self.conn,
            "graph-mempool",
            "mempool",
            "https://mempool.example/api",
            chain="bitcoin",
            network="main",
            timeout=5,
            commit=False,
        )
        self._tx("no-lookup-row", "wallet-a", "outbound", 800_000_000, txid, "{}")

        with patch("kassiber.core.transaction_graph.fetch_esplora_transaction") as fetch:
            payload = self._graph("no-lookup-row")

        fetch.assert_not_called()
        self.assertEqual(payload["supportLevel"], "graphless")
        self.assertEqual(payload["unsupportedReason"], "graphless_import")

    def test_public_lookup_string_false_does_not_fetch_graphless_row(self):
        txid = "12" * 32
        create_db_backend(
            self.conn,
            "graph-mempool",
            "mempool",
            "https://mempool.example/api",
            chain="bitcoin",
            network="main",
            timeout=5,
            commit=False,
        )
        self._tx("string-false-lookup-row", "wallet-a", "outbound", 800_000_000, txid, "{}")

        with patch("kassiber.core.transaction_graph.fetch_esplora_transaction") as fetch:
            payload = self._graph("string-false-lookup-row", allow_public_lookup="false")

        fetch.assert_not_called()
        self.assertEqual(payload["supportLevel"], "graphless")
        self.assertEqual(payload["unsupportedReason"], "graphless_import")

    def test_public_lookup_string_true_fetches_graphless_row(self):
        txid = "13" * 32
        fetched = {
            "txid": txid,
            "version": 2,
            "locktime": 0,
            "vsize": 141,
            "vin": [
                {
                    "txid": "99" * 32,
                    "vout": 1,
                    "prevout": {
                        "scriptpubkey": SCRIPT_A,
                        "scriptpubkey_type": "v0_p2wpkh",
                        "scriptpubkey_address": ADDR_A,
                        "value": 1_000_000,
                    },
                }
            ],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 900_000}],
        }
        create_db_backend(
            self.conn,
            "graph-mempool",
            "mempool",
            "https://mempool.example/api",
            chain="bitcoin",
            network="main",
            timeout=5,
            commit=False,
        )
        self._tx("string-true-lookup-row", "wallet-a", "outbound", 900_000_000, txid, "{}")

        with patch(
            "kassiber.core.transaction_graph.fetch_esplora_transaction",
            return_value=fetched,
        ) as fetch:
            payload = self._graph("string-true-lookup-row", allow_public_lookup="true")

        fetch.assert_called_once_with("https://mempool.example/api", txid, timeout=5)
        self.assertEqual(payload["supportLevel"], "full")

    def test_public_lookup_rejects_ambiguous_string(self):
        self._tx("bad-lookup-flag-row", "wallet-a", "outbound", 800_000_000, "14" * 32, "{}")

        with self.assertRaises(AppError) as raised:
            self._graph("bad-lookup-flag-row", allow_public_lookup="no")

        self.assertEqual(raised.exception.code, "validation")

    def test_public_lookup_warning_does_not_leak_backend_url(self):
        txid = "13" * 32
        secret_url = "https://token.example/api?token=super-secret"
        create_db_backend(
            self.conn,
            "graph-mempool",
            "mempool",
            secret_url,
            chain="bitcoin",
            network="main",
            timeout=5,
            commit=False,
        )
        self._tx("lookup-failure-row", "wallet-a", "outbound", 800_000_000, txid, "{}")

        with patch(
            "kassiber.core.transaction_graph.fetch_esplora_transaction",
            side_effect=RuntimeError(f"boom from {secret_url}"),
        ):
            payload = self._graph("lookup-failure-row", allow_public_lookup=True)

        serialized = json.dumps(payload)
        self.assertIn("bitcoin_reference_lookup_failed", serialized)
        self.assertNotIn(secret_url, serialized)
        self.assertNotIn("super-secret", serialized)

    def test_bitcoin_lookup_uses_wallet_network_backend(self):
        txid = "14" * 32
        self.conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = ?",
            (json.dumps({"chain": "bitcoin", "network": "testnet"}), "wallet-a"),
        )
        create_db_backend(
            self.conn,
            "main-mempool",
            "mempool",
            "https://mainnet.example/api",
            chain="bitcoin",
            network="main",
            timeout=5,
            commit=False,
        )
        create_db_backend(
            self.conn,
            "testnet-mempool",
            "mempool",
            "https://testnet.example/api",
            chain="bitcoin",
            network="testnet",
            timeout=5,
            commit=False,
        )
        fetched = {
            "txid": txid,
            "vin": [{"txid": "15" * 32, "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 9}}],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 8}],
        }
        self._tx("testnet-row", "wallet-a", "outbound", 8_000, txid, "{}")

        with patch(
            "kassiber.core.transaction_graph.fetch_esplora_transaction",
            return_value=fetched,
        ) as fetch:
            self._graph("testnet-row", allow_public_lookup=True)

        fetch.assert_called_once_with("https://testnet.example/api", txid, timeout=5)

    def test_bitcoin_lookup_normalizes_default_http_explorer_root_url(self):
        txid = "1c" * 32
        create_db_backend(
            self.conn,
            "graph-mempool-root",
            "mempool",
            "https://mempool.example",
            chain="bitcoin",
            network="main",
            timeout=5,
            commit=False,
        )
        set_setting(self.conn, "default_backend", "graph-mempool-root")
        fetched = {
            "txid": txid,
            "vin": [{"txid": "1d" * 32, "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 9}}],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 8}],
        }
        self._tx("default-http-root-row", "wallet-a", "outbound", 8_000, txid, "{}")

        with patch(
            "kassiber.core.transaction_graph.fetch_esplora_transaction",
            return_value=fetched,
        ) as fetch:
            payload = self._graph("default-http-root-row", allow_public_lookup=True)

        fetch.assert_called_once_with("https://mempool.example/api", txid, timeout=5)
        self.assertEqual(payload["supportLevel"], "full")

    def test_bitcoin_lookup_uses_runtime_default_before_persisted_default(self):
        txid = "20" * 32
        create_db_backend(
            self.conn,
            "stored-mempool",
            "mempool",
            "https://stored.example/api",
            chain="bitcoin",
            network="main",
            timeout=5,
            commit=False,
        )
        set_setting(self.conn, "default_backend", "stored-mempool")
        runtime_config = {
            "default_backend": "runtime-mempool",
            "backends": {
                "runtime-mempool": {
                    "kind": "mempool",
                    "chain": "bitcoin",
                    "network": "main",
                    "url": "https://runtime.example",
                    "timeout": 5,
                }
            },
        }
        fetched = {
            "txid": txid,
            "vin": [{"txid": "21" * 32, "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 9}}],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 8}],
        }
        self._tx("runtime-default-row", "wallet-a", "outbound", 8_000, txid, "{}")

        with patch(
            "kassiber.core.transaction_graph.fetch_esplora_transaction",
            return_value=fetched,
        ) as fetch:
            payload = self._graph(
                "runtime-default-row",
                allow_public_lookup=True,
                runtime_config=runtime_config,
            )

        fetch.assert_called_once_with("https://runtime.example/api", txid, timeout=5)
        self.assertEqual(payload["supportLevel"], "full")

    def test_bitcoin_lookup_falls_back_to_next_runtime_backend(self):
        txid = "28" * 32
        runtime_config = {
            "default_backend": "runtime-down",
            "backends": {
                "runtime-down": {
                    "kind": "mempool",
                    "chain": "bitcoin",
                    "network": "main",
                    "url": "https://down.example",
                    "timeout": 5,
                },
                "runtime-up": {
                    "kind": "mempool",
                    "chain": "bitcoin",
                    "network": "main",
                    "url": "https://up.example",
                    "timeout": 5,
                },
            },
        }
        fetched = {
            "txid": txid,
            "vin": [{"txid": "29" * 32, "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 9}}],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 8}],
        }

        def fake_fetch(url, txid_arg, **kwargs):
            if url == "https://down.example/api":
                raise RuntimeError("backend unavailable")
            self.assertEqual(url, "https://up.example/api")
            self.assertEqual(txid_arg, txid)
            self.assertEqual(kwargs, {"timeout": 5})
            return fetched

        self._tx("runtime-fallback-row", "wallet-a", "outbound", 8_000, txid, "{}")

        with patch("kassiber.core.transaction_graph.fetch_esplora_transaction", side_effect=fake_fetch) as fetch:
            payload = self._graph(
                "runtime-fallback-row",
                allow_public_lookup=True,
                runtime_config=runtime_config,
            )

        self.assertEqual(
            [call.args[0] for call in fetch.call_args_list],
            ["https://down.example/api", "https://up.example/api"],
        )
        self.assertEqual(payload["supportLevel"], "full")

    def test_http_graph_lookup_uses_backend_proxy(self):
        txid = "22" * 32
        create_db_backend(
            self.conn,
            "onion-mempool",
            "mempool",
            "http://mempoolhiddenexample.onion/api",
            chain="bitcoin",
            network="main",
            timeout=5,
            tor_proxy="socks5h://127.0.0.1:9050",
            commit=False,
        )
        fetched = {
            "txid": txid,
            "vin": [{"txid": "23" * 32, "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 9}}],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 8}],
        }
        self._tx("proxy-http-row", "wallet-a", "outbound", 8_000, txid, "{}")

        with patch(
            "kassiber.core.transaction_graph.fetch_esplora_transaction",
            return_value=fetched,
        ) as fetch:
            payload = self._graph("proxy-http-row", allow_public_lookup=True)

        fetch.assert_called_once_with(
            "http://mempoolhiddenexample.onion/api",
            txid,
            timeout=5,
            proxy_url="socks5h://127.0.0.1:9050",
        )
        self.assertEqual(payload["supportLevel"], "full")

    def test_bitcoin_lookup_prefers_default_electrum_before_http_backend(self):
        txid = "1a" * 32
        create_db_backend(
            self.conn,
            "graph-mempool",
            "mempool",
            "https://mempool.example/api",
            chain="bitcoin",
            network="main",
            timeout=5,
            commit=False,
        )
        create_db_backend(
            self.conn,
            "graph-fulcrum",
            "electrum",
            "ssl://fulcrum.example:50002",
            chain="bitcoin",
            network="main",
            timeout=60,
            commit=False,
        )
        set_setting(self.conn, "default_backend", "graph-fulcrum")
        self._tx("default-electrum-row", "wallet-a", "outbound", 900_000_000, txid, "{}")
        _FakeElectrumClient.calls = []
        _FakeElectrumClient.backends = []
        _FakeElectrumClient.responses = {txid: "current-raw"}
        decoded_current = {
            "version": 2,
            "locktime": 0,
            "vin": [],
            "vout": [{"n": 0, "script_hex": SCRIPT_B, "value_sats": 900_000}],
        }

        with patch("kassiber.core.transaction_graph.fetch_esplora_transaction") as fetch, patch(
            "kassiber.core.transaction_graph.ElectrumClient",
            _FakeElectrumClient,
        ), patch(
            "kassiber.core.transaction_graph.decode_raw_transaction",
            return_value=decoded_current,
        ):
            payload = self._graph("default-electrum-row", allow_public_lookup=True)

        fetch.assert_not_called()
        self.assertEqual(
            _FakeElectrumClient.calls,
            [("blockchain.transaction.get", (txid,))],
        )
        self.assertEqual(_FakeElectrumClient.backends[0]["timeout"], 5)
        self.assertEqual(payload["supportLevel"], "full")

    def test_runtime_electrum_graph_lookup_preserves_tls_options(self):
        txid = "24" * 32
        runtime_config = {
            "default_backend": "runtime-fulcrum",
            "backends": {
                "runtime-fulcrum": {
                    "kind": "electrum",
                    "chain": "bitcoin",
                    "network": "main",
                    "url": "ssl://fulcrum.example:50002",
                    "timeout": 60,
                    "certificate": "/tmp/fulcrum.pem",
                    "insecure": True,
                }
            },
        }
        self._tx("runtime-electrum-row", "wallet-a", "outbound", 900_000_000, txid, "{}")
        _FakeElectrumClient.calls = []
        _FakeElectrumClient.backends = []
        _FakeElectrumClient.responses = {txid: "current-raw"}
        decoded_current = {
            "version": 2,
            "locktime": 0,
            "vin": [],
            "vout": [{"n": 0, "script_hex": SCRIPT_B, "value_sats": 900_000}],
        }

        with patch("kassiber.core.transaction_graph.ElectrumClient", _FakeElectrumClient), patch(
            "kassiber.core.transaction_graph.decode_raw_transaction",
            return_value=decoded_current,
        ):
            payload = self._graph(
                "runtime-electrum-row",
                allow_public_lookup=True,
                runtime_config=runtime_config,
            )

        self.assertEqual(_FakeElectrumClient.backends[0]["timeout"], 5)
        self.assertEqual(_FakeElectrumClient.backends[0]["certificate"], "/tmp/fulcrum.pem")
        self.assertIs(_FakeElectrumClient.backends[0]["insecure"], True)
        self.assertEqual(payload["supportLevel"], "full")

    def test_runtime_bitcoinrpc_graph_lookup_preserves_top_level_db_config_fields(self):
        txid = "26" * 32
        prev_txid = "27" * 32
        runtime_config = {
            "default_backend": "runtime-core",
            "backends": {
                "runtime-core": {
                    "kind": "bitcoinrpc",
                    "chain": "bitcoin",
                    "network": "main",
                    "url": "http://127.0.0.1:18443",
                    "timeout": 5,
                    "username": "rpc-user",
                    "password": "rpc-password",
                    "walletprefix": "demo-book",
                }
            },
        }
        decoded_current = {
            "txid": txid,
            "version": 2,
            "locktime": 0,
            "size": 191,
            "vsize": 110,
            "weight": 437,
            "vin": [
                {
                    "txid": prev_txid,
                    "vout": 0,
                    "sequence": 0xFFFFFFFD,
                    "prevout": {
                        "n": 0,
                        "value": 0.001,
                        "scriptPubKey": {
                            "hex": SCRIPT_A,
                            "type": "witness_v0_keyhash",
                            "address": ADDR_A,
                        },
                    },
                }
            ],
            "vout": [
                {
                    "n": 0,
                    "value": 0.0009,
                    "scriptPubKey": {
                        "hex": SCRIPT_B,
                        "type": "witness_v0_keyhash",
                        "address": ADDR_B,
                    },
                }
            ],
        }
        self._tx("runtime-core-row", "wallet-a", "outbound", 90_000_000, txid, "{}")

        with patch(
            "kassiber.core.transaction_graph.bitcoinrpc_call",
            return_value=decoded_current,
        ) as rpc_call:
            payload = self._graph(
                "runtime-core-row",
                allow_public_lookup=True,
                runtime_config=runtime_config,
            )

        rpc_call.assert_called_once()
        backend = rpc_call.call_args.args[0]
        self.assertEqual(backend["username"], "rpc-user")
        self.assertEqual(backend["password"], "rpc-password")
        self.assertEqual(backend["walletprefix"], "demo-book")
        self.assertEqual(payload["supportLevel"], "full")

    def test_runtime_graph_lookup_prefers_user_backend_over_implicit_builtin_default(self):
        txid = "28" * 32
        runtime_config = {
            "default_backend": "fulcrum",
            "bootstrap_default_backend": "fulcrum",
            "default_backend_source": "built-in default",
            "process_env_overrides": {"backends": {}, "default_backend": False},
            "dotenv_backends": [],
            "backends": {
                **{name: dict(config) for name, config in DEFAULT_BACKENDS.items()},
                "own-mempool": {
                    "kind": "mempool",
                    "chain": "bitcoin",
                    "network": "main",
                    "url": "https://own.example",
                    "timeout": 5,
                    "source": "database",
                },
            },
        }
        fetched = {
            "txid": txid,
            "vin": [{"txid": "29" * 32, "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 9}}],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 8}],
        }
        self._tx("runtime-user-backend-row", "wallet-a", "outbound", 8_000, txid, "{}")

        _FakeElectrumClient.calls = []
        with patch(
            "kassiber.core.transaction_graph.fetch_esplora_transaction",
            return_value=fetched,
        ) as fetch, patch("kassiber.core.transaction_graph.ElectrumClient", _FakeElectrumClient):
            payload = self._graph(
                "runtime-user-backend-row",
                allow_public_lookup=True,
                runtime_config=runtime_config,
            )

        fetch.assert_called_once_with("https://own.example/api", txid, timeout=5)
        self.assertEqual(_FakeElectrumClient.calls, [])
        self.assertEqual(payload["supportLevel"], "full")

    def test_runtime_graph_lookup_keeps_explicit_builtin_default(self):
        txid = "2a" * 32
        runtime_config = {
            "default_backend": "mempool",
            "bootstrap_default_backend": "fulcrum",
            "default_backend_source": "built-in default",
            "process_env_overrides": {"backends": {}, "default_backend": False},
            "dotenv_backends": [],
            "backends": {
                **{name: dict(config) for name, config in DEFAULT_BACKENDS.items()},
                "own-mempool": {
                    "kind": "mempool",
                    "chain": "bitcoin",
                    "network": "main",
                    "url": "https://own.example/api",
                    "timeout": 5,
                    "source": "database",
                },
            },
        }
        fetched = {
            "txid": txid,
            "vin": [{"txid": "2b" * 32, "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 9}}],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 8}],
        }
        self._tx("runtime-explicit-builtin-row", "wallet-a", "outbound", 8_000, txid, "{}")

        with patch(
            "kassiber.core.transaction_graph.fetch_esplora_transaction",
            return_value=fetched,
        ) as fetch:
            payload = self._graph(
                "runtime-explicit-builtin-row",
                allow_public_lookup=True,
                runtime_config=runtime_config,
            )

        fetch.assert_called_once_with(DEFAULT_BACKENDS["mempool"]["url"], txid, timeout=5)
        self.assertEqual(payload["supportLevel"], "full")

    def test_liquid_lookup_skips_implicit_builtin_runtime_backends(self):
        txid = "25" * 32
        runtime_config = {
            "default_backend": "fulcrum",
            "backends": {name: dict(config) for name, config in DEFAULT_BACKENDS.items()},
        }
        self._tx(
            "implicit-liquid-runtime-row",
            "wallet-a",
            "outbound",
            900_000_000,
            txid,
            "{}",
            asset="LBTC",
        )
        _FakeElectrumClient.calls = []

        with patch("kassiber.core.transaction_graph.ElectrumClient", _FakeElectrumClient):
            payload = self._graph(
                "implicit-liquid-runtime-row",
                allow_public_lookup=True,
                runtime_config=runtime_config,
            )

        self.assertEqual(_FakeElectrumClient.calls, [])
        self.assertEqual(payload["supportLevel"], "graphless")
        warning_codes = {warning["code"] for warning in payload["warnings"]}
        self.assertIn("liquid_reference_lookup_unavailable", warning_codes)

    def test_bitcoin_electrum_prevtx_lookup_reuses_cached_prev_transaction(self):
        txid = "16" * 32
        prev_txid = "17" * 32
        tg._store_graph_lookup_cache(
            self.conn,
            "bitcoin",
            "main",
            prev_txid,
            {
                "txid": prev_txid,
                "vin": [],
                "vout": [
                    {"n": 0, "scriptpubkey": SCRIPT_A, "value": 700_000},
                ],
            },
        )
        create_db_backend(
            self.conn,
            "graph-fulcrum",
            "electrum",
            "ssl://fulcrum.example:50002",
            chain="bitcoin",
            network="main",
            timeout=60,
            commit=False,
        )
        self._tx("electrum-row", "wallet-a", "outbound", 600_000_000, txid, "{}")
        _FakeElectrumClient.calls = []
        _FakeElectrumClient.backends = []
        _FakeElectrumClient.responses = {txid: "current-raw"}

        decoded_current = {
            "version": 2,
            "locktime": 0,
            "vin": [{"txid": prev_txid, "vout": 0}],
            "vout": [{"n": 0, "script_hex": SCRIPT_B, "value_sats": 600_000}],
        }
        with patch("kassiber.core.transaction_graph.ElectrumClient", _FakeElectrumClient), patch(
            "kassiber.core.transaction_graph.decode_raw_transaction",
            return_value=decoded_current,
        ) as decode:
            payload = self._graph("electrum-row", allow_public_lookup=True)

        self.assertEqual(
            _FakeElectrumClient.calls,
            [("blockchain.transaction.get", (txid,))],
        )
        self.assertEqual(_FakeElectrumClient.backends[0]["timeout"], 5)
        decode.assert_called_once_with("current-raw")
        self.assertEqual(payload["supportLevel"], "full")
        self.assertEqual(payload["inputs"][0]["outpoint"], f"{prev_txid}:0")
        self.assertEqual(payload["inputs"][0]["valueSats"], 700_000)
        self.assertEqual(payload["outputs"][0]["valueSats"], 600_000)
        self.assertEqual(payload["fee"]["valueSats"], 100_000)

    def test_bitcoin_electrum_prevtx_fan_in_limit_surfaces_partial_without_batch_fetch(self):
        txid = "1b" * 32
        create_db_backend(
            self.conn,
            "graph-fulcrum",
            "electrum",
            "ssl://fulcrum.example:50002",
            chain="bitcoin",
            network="main",
            timeout=60,
            commit=False,
        )
        self._tx("fan-in-limit-row", "wallet-a", "outbound", 1_000_000_000, txid, "{}")
        prev_txids = [f"{index:064x}" for index in range(tg.MAX_ELECTRUM_GRAPH_PREVTX_LOOKUPS + 1)]
        decoded_current = {
            "version": 2,
            "locktime": 0,
            "vin": [{"txid": prev_txid, "vout": 0} for prev_txid in prev_txids],
            "vout": [{"n": 0, "script_hex": SCRIPT_B, "value_sats": 1_000_000}],
        }
        _FakeElectrumClient.calls = []
        _FakeElectrumClient.backends = []
        _FakeElectrumClient.responses = {txid: "current-raw"}

        with patch("kassiber.core.transaction_graph.ElectrumClient", _FakeElectrumClient), patch(
            "kassiber.core.transaction_graph.decode_raw_transaction",
            return_value=decoded_current,
        ) as decode:
            payload = self._graph("fan-in-limit-row", allow_public_lookup=True)

        self.assertEqual(
            _FakeElectrumClient.calls,
            [("blockchain.transaction.get", (txid,))],
        )
        decode.assert_called_once_with("current-raw")
        self.assertEqual(_FakeElectrumClient.backends[0]["timeout"], 5)
        self.assertEqual(payload["supportLevel"], "partial")
        self.assertEqual(payload["unsupportedReason"], "input_prevout_values_missing")
        warning_codes = {warning["code"] for warning in payload["warnings"]}
        self.assertIn("bitcoin_reference_lookup_prevout_limit", warning_codes)

    def test_bitcoin_electrum_coinbase_sentinel_input_does_not_fetch_zero_prevtx(self):
        txid = "1e" * 32
        create_db_backend(
            self.conn,
            "graph-fulcrum",
            "electrum",
            "ssl://fulcrum.example:50002",
            chain="bitcoin",
            network="main",
            timeout=5,
            commit=False,
        )
        self._tx("coinbase-electrum-row", "wallet-a", "inbound", 625_000_000_000, txid, "{}")
        _FakeElectrumClient.calls = []
        _FakeElectrumClient.responses = {txid: "coinbase-raw"}
        decoded_current = {
            "version": 2,
            "locktime": 0,
            "vin": [{"txid": "00" * 32, "vout": 0xFFFFFFFF}],
            "vout": [{"n": 0, "script_hex": SCRIPT_A, "value_sats": 625_000_000}],
        }

        with patch("kassiber.core.transaction_graph.ElectrumClient", _FakeElectrumClient), patch(
            "kassiber.core.transaction_graph.decode_raw_transaction",
            return_value=decoded_current,
        ):
            payload = self._graph("coinbase-electrum-row", allow_public_lookup=True)

        self.assertEqual(
            _FakeElectrumClient.calls,
            [("blockchain.transaction.get", (txid,))],
        )
        self.assertEqual(payload["supportLevel"], "partial")
        self.assertEqual(payload["unsupportedReason"], "input_prevout_values_missing")
        cached = self._cached_graph_raw(txid)
        self.assertNotIn("txid", cached["vin"][0])
        self.assertNotIn("vout", cached["vin"][0])

    def test_bitcoin_electrum_duplicate_prevtx_inputs_fetch_once_and_cache_complete_graph(self):
        txid = "18" * 32
        prev_txid = "19" * 32
        create_db_backend(
            self.conn,
            "graph-fulcrum",
            "electrum",
            "ssl://fulcrum.example:50002",
            chain="bitcoin",
            network="main",
            timeout=5,
            commit=False,
        )
        self._tx("duplicate-prev-row", "wallet-a", "outbound", 1_900_000_000, txid, "{}")
        _FakeElectrumClient.calls = []
        _FakeElectrumClient.responses = {
            txid: "current-raw",
            prev_txid: "prev-raw",
        }
        decoded_current = {
            "version": 2,
            "locktime": 0,
            "vin": [
                {"txid": prev_txid, "vout": 0},
                {"txid": prev_txid, "vout": 1},
            ],
            "vout": [{"n": 0, "script_hex": SCRIPT_B, "value_sats": 1_900_000}],
        }
        decoded_prev = {
            "version": 2,
            "locktime": 0,
            "vin": [],
            "vout": [
                {"n": 0, "script_hex": SCRIPT_A, "value_sats": 1_200_000},
                {"n": 1, "script_hex": SCRIPT_C, "value_sats": 800_000},
            ],
        }

        def decode(raw_hex):
            if raw_hex == "current-raw":
                return decoded_current
            if raw_hex == "prev-raw":
                return decoded_prev
            raise AssertionError(f"Unexpected raw tx decode: {raw_hex}")

        with patch("kassiber.core.transaction_graph.ElectrumClient", _FakeElectrumClient), patch(
            "kassiber.core.transaction_graph.decode_raw_transaction",
            side_effect=decode,
        ) as decode_spy:
            first = self._graph("duplicate-prev-row", allow_public_lookup=True)

        self.assertEqual(
            _FakeElectrumClient.calls,
            [
                ("blockchain.transaction.get", (txid,)),
                ("blockchain.transaction.get", (prev_txid,)),
            ],
        )
        self.assertEqual(decode_spy.call_count, 2)
        self.assertEqual(first["supportLevel"], "full")
        self.assertIsNone(first["unsupportedReason"])
        self.assertEqual(first["transaction"]["inputCount"], 2)
        self.assertEqual(first["transaction"]["outputCount"], 1)
        self.assertEqual([node["valueSats"] for node in first["inputs"]], [1_200_000, 800_000])
        self.assertEqual(first["fee"]["valueSats"], 100_000)

        cached = self._cached_graph_raw(txid)
        self.assertEqual([vin["prevout"]["value"] for vin in cached["vin"]], [1_200_000, 800_000])
        cached_serialized = json.dumps(cached, sort_keys=True)
        self.assertNotIn("current-raw", cached_serialized)
        self.assertNotIn("prev-raw", cached_serialized)

        _FakeElectrumClient.calls = []
        _FakeElectrumClient.responses = {}
        with patch("kassiber.core.transaction_graph.ElectrumClient", _FakeElectrumClient), patch(
            "kassiber.core.transaction_graph.decode_raw_transaction"
        ) as decode_cached:
            second = self._graph("duplicate-prev-row", allow_public_lookup=True)

        self.assertEqual(_FakeElectrumClient.calls, [])
        decode_cached.assert_not_called()
        self.assertEqual(second["supportLevel"], "full")
        self.assertEqual([node["valueSats"] for node in second["inputs"]], [1_200_000, 800_000])

    def test_multi_source_consolidation_annotation(self):
        self._utxo("wallet-a", ADDR_A, "prev-a", 0, amount=51_000_000)
        self._utxo("wallet-b", ADDR_B, "prev-b", 0, amount=31_000_000)
        self._utxo("wallet-c", ADDR_C, "scan-c", 0, amount=81_000_000)
        raw = {
            "txid": "consol",
            "vin": [
                {"txid": "prev-a", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 51_000_000}},
                {"txid": "prev-b", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_B, "value": 31_000_000}},
            ],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_C, "scriptpubkey_address": ADDR_C, "value": 81_000_000}],
        }
        self._tx("a-out", "wallet-a", "outbound", 50_000_000_000, "consol", raw, fee_msat=1_000_000_000)
        self._tx("b-out", "wallet-b", "outbound", 30_000_000_000, "consol", raw, fee_msat=1_000_000_000)
        self._tx("c-in", "wallet-c", "inbound", 81_000_000_000, "consol", "{}")

        payload = self._graph("a-out")

        codes = {annotation["code"] for annotation in payload["annotations"]}
        self.assertIn("multi_source_consolidation", codes)
        self.assertIn("multi-consol:consol", payload["accounting"]["transferGroupIds"])

    def test_partial_external_residual_annotation(self):
        self._utxo("wallet-a", ADDR_A, "prev-partial", 0, amount=61_000_000)
        self._utxo("wallet-b", ADDR_B, "scan-b", 0, amount=50_000_000)
        raw = {
            "txid": "partial",
            "vin": [{"txid": "prev-partial", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 61_000_000}}],
            "vout": [
                {"n": 0, "scriptpubkey": SCRIPT_B, "value": 50_000_000},
                {"n": 1, "scriptpubkey": "00142222222222222222222222222222222222222222", "value": 10_000_000},
            ],
        }
        self._tx("partial-out", "wallet-a", "outbound", 60_000_000_000, "partial", raw, fee_msat=1_000_000_000)

        payload = self._graph("partial-out")

        codes = {annotation["code"] for annotation in payload["annotations"]}
        self.assertIn("ownership_derived", codes)
        self.assertIn("partial_external_residual", codes)

    def test_ambiguous_destination_receipt_warns(self):
        self._utxo("wallet-a", ADDR_A, "prev-ambig", 0, amount=51_000_000)
        self._utxo("wallet-b", ADDR_B, "scan-ambig", 0, amount=50_000_000)
        raw = {
            "txid": "ambig",
            "vin": [{"txid": "prev-ambig", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 51_000_000}}],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 50_000_000}],
        }
        self._tx("ambig-out", "wallet-a", "outbound", 50_000_000_000, "ambig", raw, fee_msat=1_000_000_000)
        self._tx("ambig-in-1", "wallet-b", "inbound", 50_000_000_000, "ambig", "{}")
        self._tx("ambig-in-2", "wallet-b", "inbound", 50_000_000_000, "ambig", "{}")

        payload = self._graph("ambig-out")

        codes = {warning["code"] for warning in payload["warnings"]}
        self.assertIn("ownership_transfer_destination_ambiguous", codes)

    def test_mixed_case_recorded_fanout_is_annotated(self):
        txid = "ABCDEF" + "1" * 58
        self._tx("fan-out", "wallet-a", "outbound", 80_000_000_000, txid, "{}")
        self._tx("fan-in-b", "wallet-b", "inbound", 50_000_000_000, txid.lower(), "{}")
        self._tx("fan-in-c", "wallet-c", "inbound", 30_000_000_000, txid.upper(), "{}")

        payload = self._graph("fan-out")

        self.assertEqual(payload["supportLevel"], "graphless")
        codes = {annotation["code"] for annotation in payload["annotations"]}
        self.assertIn("recorded_fanout", codes)

    def test_recorded_one_to_one_transfer_is_annotated(self):
        txid = "abcd" + "2" * 60
        self._tx("one-out", "wallet-a", "outbound", 50_000_000_000, txid, "{}")
        self._tx("one-in", "wallet-b", "inbound", 50_000_000_000, txid.upper(), "{}")

        payload = self._graph("one-out")

        self.assertEqual(payload["supportLevel"], "graphless")
        codes = {annotation["code"] for annotation in payload["annotations"]}
        self.assertIn("recorded_self_transfer", codes)
        self.assertIn(
            f"recorded-self-transfer:{txid.lower()}",
            payload["accounting"]["transferGroupIds"],
        )

    def test_excluded_rows_are_ignored_for_graph_semantics(self):
        txid = "cdef" + "3" * 60
        self._tx("excluded-fan-out", "wallet-a", "outbound", 80_000_000_000, txid, "{}")
        self._tx("excluded-fan-in-b", "wallet-b", "inbound", 50_000_000_000, txid, "{}")
        self._tx("excluded-fan-in-c", "wallet-c", "inbound", 30_000_000_000, txid, "{}")
        self.conn.execute(
            "UPDATE transactions SET excluded = 1 WHERE id = ?",
            ("excluded-fan-in-c",),
        )

        payload = self._graph("excluded-fan-out")

        codes = {annotation["code"] for annotation in payload["annotations"]}
        self.assertIn("recorded_self_transfer", codes)
        self.assertNotIn("recorded_fanout", codes)

    def test_manual_pair_ids_suppress_graph_derivation(self):
        self._utxo("wallet-a", ADDR_A, "manual-prev", 0, amount=51_000_000)
        self._utxo("wallet-b", ADDR_B, "manual-scan", 0, amount=50_000_000)
        raw = {
            "txid": "manual-pair",
            "vin": [{"txid": "manual-prev", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 51_000_000}}],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 50_000_000}],
        }
        self._tx("manual-out", "wallet-a", "outbound", 50_000_000_000, "manual-pair", raw, fee_msat=1_000_000_000)
        self._tx("manual-in", "wallet-b", "inbound", 50_000_000_000, "manual-pair", "{}")
        self.conn.execute(
            """
            INSERT INTO transaction_pairs(
                id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
                kind, policy, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "manual-pair-1",
                "ws-1",
                "profile-1",
                "manual-out",
                "manual-in",
                "manual",
                "carrying-value",
                NOW,
            ),
        )

        payload = self._graph("manual-out")

        codes = {annotation["code"] for annotation in payload["annotations"]}
        self.assertNotIn("ownership_derived", codes)
        self.assertNotIn("recorded_self_transfer", codes)

    def test_ambiguous_owned_output_is_not_labeled_change(self):
        self._utxo("wallet-a", ADDR_A, "ambig-prev", 0, amount=1_000_000)
        self._utxo("wallet-a", ADDR_B, "ambig-output", 0, amount=900_000)
        self._utxo("wallet-b", ADDR_B, "ambig-output", 0, amount=900_000)
        raw = {
            "txid": "ambig-output",
            "vin": [{"txid": "ambig-prev", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 1_000_000}}],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "scriptpubkey_address": ADDR_B, "value": 900_000}],
        }
        self._tx("ambig-change-row", "wallet-a", "outbound", 900_000_000, "ambig-output", raw)

        payload = self._graph("ambig-change-row")

        self.assertEqual(payload["outputs"][0]["role"], "ambiguous_owned_output")

    def test_graph_ownership_matches_are_filtered_by_network(self):
        self.conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = ?",
            (json.dumps({"chain": "bitcoin", "network": "testnet"}), "wallet-b"),
        )
        self._utxo("wallet-a", ADDR_A, "net-prev", 0, amount=1_000_000)
        self._utxo(
            "wallet-b",
            ADDR_B,
            "testnet-owned-output",
            0,
            amount=900_000,
            network="testnet",
        )
        raw = {
            "txid": "network-filter-tx",
            "vin": [
                {
                    "txid": "net-prev",
                    "vout": 0,
                    "prevout": {"scriptpubkey": SCRIPT_A, "value": 1_000_000},
                }
            ],
            "vout": [
                {
                    "n": 0,
                    "scriptpubkey": SCRIPT_B,
                    "scriptpubkey_address": ADDR_B,
                    "value": 900_000,
                }
            ],
        }
        self._tx(
            "network-filter-row",
            "wallet-a",
            "outbound",
            900_000_000,
            "network-filter-tx",
            raw,
        )

        payload = self._graph("network-filter-row")

        self.assertEqual(payload["outputs"][0]["ownership"], "external")
        self.assertEqual(payload["outputs"][0]["role"], "external_recipient")

    def test_reviewed_swap_pair_route_is_curated(self):
        raw = {
            "txid": "liquid-swap-out",
            "vin": [
                {"txid": "liquid-prev-a", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}},
                {"txid": "liquid-prev-b", "vout": 1, "prevout": {"scriptpubkey": SCRIPT_A}},
            ],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "valuecommitment": "09" + "aa" * 32}],
        }
        self._tx(
            "swap-out",
            "wallet-a",
            "outbound",
            124_262_750_000,
            "liquid-swap-out",
            raw,
            fee_msat=129_770_000,
            asset="LBTC",
            kind="swap",
            description="Liquid spend to swap address",
            counterparty="Swap LBTC -> BTC",
        )
        self._tx(
            "swap-in",
            "wallet-b",
            "inbound",
            124_132_980_000,
            "bitcoin-swap-receive",
            "{}",
            asset="BTC",
            kind="swap",
            description="Bitcoin receive from swap",
            counterparty="Swap LBTC -> BTC",
        )
        self.conn.execute(
            """
            INSERT INTO transaction_pairs(
                id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
                kind, policy, notes, swap_fee_msat, swap_fee_kind, confidence_at_pair,
                pair_source, out_amount, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pair-swap-1",
                "ws-1",
                "profile-1",
                "swap-out",
                "swap-in",
                "swap",
                "carrying-value",
                "reviewed cross-chain swap",
                129_770_000,
                "network_or_provider_fee",
                "manual",
                "manual",
                124_262_750_000,
                NOW,
            ),
        )

        payload = self._graph("swap-out")

        route = payload["swapRoute"]
        self.assertIsNotNone(route)
        self.assertEqual(route["routeKind"], "swap")
        self.assertEqual(route["currentLeg"], "out")
        self.assertEqual(route["out"]["asset"], "LBTC")
        self.assertEqual(route["out"]["network"], "Liquid")
        self.assertEqual(route["out"]["role"], "consolidation")
        self.assertEqual(route["out"]["wallet"]["label"], "Cold")
        self.assertEqual(route["in"]["asset"], "BTC")
        self.assertEqual(route["in"]["network"], "Bitcoin")
        self.assertEqual(route["in"]["role"], "receive")
        self.assertEqual(route["in"]["wallet"]["label"], "Hot")
        self.assertEqual(route["swapFeeMsat"], 129_770_000)
        serialized = json.dumps(payload)
        self.assertNotIn("reviewed cross-chain swap", serialized)
        self.assertNotIn("raw_json", serialized)

    def test_manual_coinjoin_pair_routes_as_coinjoin(self):
        self._tx(
            "coinjoin-out",
            "wallet-a",
            "outbound",
            100_000_000_000,
            "coinjoin-out-tx",
            {
                "txid": "coinjoin-out-tx",
                "vin": [{"txid": "coinjoin-prev", "vout": 0, "prevout": {"value": 100_000_000}}],
                "vout": [{"n": 0, "value": 99_900_000}],
            },
            fee_msat=100_000_000,
            kind="withdrawal",
            counterparty="Manual privacy wallet",
        )
        self._tx(
            "coinjoin-in",
            "wallet-b",
            "inbound",
            99_500_000_000,
            "coinjoin-in-tx",
            {"txid": "coinjoin-in-tx", "vin": [], "vout": [{"n": 0, "value": 99_500_000}]},
            kind="deposit",
            counterparty="Manual privacy wallet",
        )
        self.conn.execute(
            """
            INSERT INTO transaction_pairs(
                id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
                kind, policy, notes, swap_fee_msat, swap_fee_kind, confidence_at_pair,
                pair_source, out_amount, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pair-coinjoin-1",
                "ws-1",
                "profile-1",
                "coinjoin-out",
                "coinjoin-in",
                "coinjoin",
                "carrying-value",
                "reviewed generic Coinjoin hop",
                None,
                None,
                "manual",
                "manual",
                None,
                NOW,
            ),
        )

        payload = self._graph("coinjoin-out")

        route = payload["swapRoute"]
        self.assertEqual(route["kind"], "coinjoin")
        self.assertEqual(route["routeKind"], "coinjoin")
        self.assertEqual(route["currentLeg"], "out")
        self.assertEqual(route["out"]["asset"], "BTC")
        self.assertEqual(route["in"]["asset"], "BTC")
        self.assertNotIn("reviewed generic Coinjoin hop", json.dumps(payload))

    def test_payload_does_not_leak_secret_bearing_fields(self):
        self._utxo("wallet-a", ADDR_A, "prevsecret", 0, amount=2_000_000)
        raw = {
            "txid": "secret-tx",
            "descriptor": "wpkh([fingerprint/84h]xpubSECRET/0/*)",
            "backend_url": "https://secret.example",
            "token": "super-secret-token",
            "vin": [{"txid": "prevsecret", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 2_000_000}}],
            "vout": [{"n": 0, "scriptpubkey": "00143333333333333333333333333333333333333333", "value": 1_900_000}],
        }
        self._tx("secret-out", "wallet-a", "outbound", 1_900_000_000, "secret-tx", raw, fee_msat=100_000_000)

        payload = self._graph("secret-out")

        serialized = json.dumps(payload)
        self.assertNotIn("xpubSECRET", serialized)
        self.assertNotIn("super-secret-token", serialized)
        self.assertNotIn("backend_url", serialized)
        self.assertNotIn(SCRIPT_A, serialized)

    def test_liquid_explicit_fee_output_is_classified_as_fee(self):
        txid = "ab" * 32
        raw = {
            "txid": txid,
            "version": 2,
            "locktime": 0,
            "vsize": 200,
            "vin": [
                {
                    "txid": "cd" * 32,
                    "vout": 0,
                    "prevout": {
                        "scriptpubkey": SCRIPT_A,
                        "valuecommitment": "09" + "11" * 32,
                        "assetcommitment": "0a" + "22" * 32,
                    },
                }
            ],
            "vout": [
                {
                    "n": 0,
                    "scriptpubkey": SCRIPT_B,
                    "valuecommitment": "09" + "33" * 32,
                    "assetcommitment": "0a" + "44" * 32,
                },
                {"n": 1, "scriptpubkey": "", "scriptpubkey_type": "fee", "value": 250},
            ],
        }
        self._tx("liquid-fee-row", "wallet-a", "outbound", 11_000_000, txid, raw, asset="LBTC")

        payload = self._graph("liquid-fee-row")

        # The unblinded fee output is the network fee, not a phantom OP_RETURN.
        self.assertEqual(payload["supportLevel"], "partial")
        self.assertEqual(payload["unsupportedReason"], "confidential_values_hidden")
        self.assertEqual(payload["transaction"]["outputCount"], 1)
        self.assertEqual([node["role"] for node in payload["outputs"]], ["external_recipient"])
        self.assertNotIn("op_return", [node.get("role") for node in payload["outputs"]])
        self.assertEqual(payload["fee"]["valueSats"], 250)
        self.assertEqual(payload["fee"]["valueBtc"], 250 / 100_000_000)
        # Fee + fee rate are recoverable even though every value-bearing leg is blinded.
        self.assertEqual(payload["transaction"]["feeRateSatVb"], 1.25)

    def test_large_fanout_outputs_are_capped_with_overflow_node(self):
        fanout = 300
        raw = {
            "txid": "fanout-tx",
            "version": 2,
            "locktime": 0,
            "vin": [
                {"txid": "ef" * 32, "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 100_000_000}}
            ],
            "vout": [
                {"n": index, "scriptpubkey": SCRIPT_C, "value": 1_000}
                for index in range(fanout)
            ],
        }
        self._tx("fanout-row", "wallet-a", "outbound", 1_000_000_000, "fanout-tx", raw)

        payload = self._graph("fanout-row")

        # The true count is preserved in metadata, but the payload is bounded.
        self.assertEqual(payload["transaction"]["outputCount"], fanout)
        self.assertEqual(len(payload["outputs"]), 250)
        overflow = payload["outputs"][-1]
        self.assertTrue(overflow["overflow"])
        self.assertEqual(overflow["role"], "overflow")
        hidden = fanout - (250 - 1)
        self.assertEqual(overflow["overflowCount"], hidden)
        self.assertEqual(overflow["valueSats"], hidden * 1_000)

    def test_whole_number_btc_float_is_scaled_to_sats(self):
        # Regression: a round BTC float (1.0) must not be read as 1 sat.
        raw = {
            "txid": "whole-btc-tx",
            "vin": [
                {"txid": "1a" * 32, "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 2.0}}
            ],
            "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 1.0}],
        }
        self._tx("whole-btc-row", "wallet-a", "outbound", 100_000_000_000, "whole-btc-tx", raw)

        payload = self._graph("whole-btc-row")

        self.assertEqual(payload["outputs"][0]["valueSats"], 100_000_000)
        self.assertEqual(payload["outputs"][0]["valueBtc"], 1.0)

    def test_profile_semantics_cache_reuses_bundle_until_version_bumps(self):
        self._tx("cache-row", "wallet-a", "outbound", 1_000_000_000, "cache-tx", "{}")
        self.conn.commit()
        cache: dict = {}

        with patch.object(
            tg, "_compute_profile_semantics", wraps=tg._compute_profile_semantics
        ) as spy:
            tg.build_transaction_graph_snapshot(
                self.conn, {"transaction": "cache-row"}, semantics_cache=cache
            )
            tg.build_transaction_graph_snapshot(
                self.conn, {"transaction": "cache-row"}, semantics_cache=cache
            )
            # Second call reuses the cached bundle for the unchanged profile.
            self.assertEqual(spy.call_count, 1)

            self.conn.execute(
                "UPDATE profiles SET journal_input_version = journal_input_version + 1 WHERE id = ?",
                ("profile-1",),
            )
            self.conn.commit()
            tg.build_transaction_graph_snapshot(
                self.conn, {"transaction": "cache-row"}, semantics_cache=cache
            )
            # A version bump invalidates the cache and forces a recompute.
            self.assertEqual(spy.call_count, 2)

    def test_profile_semantics_recompute_every_call_without_cache(self):
        self._tx("nocache-row", "wallet-a", "outbound", 1_000_000_000, "nocache-tx", "{}")
        self.conn.commit()

        with patch.object(
            tg, "_compute_profile_semantics", wraps=tg._compute_profile_semantics
        ) as spy:
            tg.build_transaction_graph_snapshot(self.conn, {"transaction": "nocache-row"})
            tg.build_transaction_graph_snapshot(self.conn, {"transaction": "nocache-row"})
            self.assertEqual(spy.call_count, 2)

    def test_profile_semantics_cache_invalidates_on_owned_set_change(self):
        # Adding a wallet or observing a UTXO changes the owned index but does not
        # bump journal_input_version; the cache key must still notice the change.
        self._tx("ownedset-row", "wallet-a", "outbound", 1_000_000_000, "ownedset-tx", "{}")
        self.conn.commit()
        cache: dict = {}

        with patch.object(
            tg, "_compute_profile_semantics", wraps=tg._compute_profile_semantics
        ) as spy:
            tg.build_transaction_graph_snapshot(
                self.conn, {"transaction": "ownedset-row"}, semantics_cache=cache
            )
            self.assertEqual(spy.call_count, 1)

            self.conn.execute(
                """
                INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, config_json, created_at)
                VALUES(?, ?, ?, ?, ?, ?, ?, ?)
                """,
                ("wallet-new", "ws-1", "profile-1", "acct-1", "New", "custom", "{}", NOW),
            )
            self.conn.commit()
            tg.build_transaction_graph_snapshot(
                self.conn, {"transaction": "ownedset-row"}, semantics_cache=cache
            )
            self.assertEqual(spy.call_count, 2)

            self._utxo("wallet-a", ADDR_A, "newutxo", 0)
            self.conn.commit()
            tg.build_transaction_graph_snapshot(
                self.conn, {"transaction": "ownedset-row"}, semantics_cache=cache
            )
            self.assertEqual(spy.call_count, 3)

    def test_profile_semantics_cache_invalidates_on_inventory_reattribution(self):
        # An inventory re-sync can rewrite an existing outpoint's address/derivation
        # in place (UPSERT), restamping last_seen_at without changing the row count.
        self._tx("reattr-row", "wallet-a", "outbound", 1_000_000_000, "reattr-tx", "{}")
        self._utxo("wallet-a", ADDR_A, "reattr-utxo", 0)
        self.conn.commit()
        cache: dict = {}

        with patch.object(
            tg, "_compute_profile_semantics", wraps=tg._compute_profile_semantics
        ) as spy:
            tg.build_transaction_graph_snapshot(
                self.conn, {"transaction": "reattr-row"}, semantics_cache=cache
            )
            self.assertEqual(spy.call_count, 1)

            self.conn.execute(
                "UPDATE wallet_utxos SET last_seen_at = ?, address = ? WHERE txid = ?",
                ("2026-02-02T00:00:00Z", ADDR_B, "reattr-utxo"),
            )
            self.conn.commit()
            tg.build_transaction_graph_snapshot(
                self.conn, {"transaction": "reattr-row"}, semantics_cache=cache
            )
            self.assertEqual(spy.call_count, 2)

    def test_capped_overflow_with_missing_values_stays_amountless(self):
        vout = [{"n": index, "scriptpubkey": SCRIPT_C, "value": 1_000} for index in range(259)]
        vout.append({"n": 259, "scriptpubkey": SCRIPT_C})  # no value -> missing leg
        raw = {
            "txid": "fanout-missing-tx",
            "vin": [
                {"txid": "ef" * 32, "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A, "value": 100_000_000}}
            ],
            "vout": vout,
        }
        self._tx("fanout-missing-row", "wallet-a", "outbound", 1_000_000_000, "fanout-missing-tx", raw)

        payload = self._graph("fanout-missing-row")

        self.assertEqual(len(payload["outputs"]), 250)
        overflow = payload["outputs"][-1]
        self.assertTrue(overflow["overflow"])
        self.assertEqual(overflow["overflowCount"], 11)
        # A partial sum must not masquerade as the full aggregate.
        self.assertNotIn("valueSats", overflow)
        self.assertNotIn("valueBtc", overflow)


if __name__ == "__main__":
    unittest.main()
