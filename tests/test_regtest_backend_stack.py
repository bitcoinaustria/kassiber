from __future__ import annotations

import os
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from dev.regtest import backend_stack


class RegtestBackendStackTest(unittest.TestCase):
    def test_rpc_client_uses_elements_env_with_bitcoin_credential_fallback(self) -> None:
        with patch.dict(
            os.environ,
            {
                "BITCOIN_RPC_USER": "fallback-user",
                "BITCOIN_RPC_PASSWORD": "fallback-pass",
                "ELEMENTS_RPC_URL": "http://elements.example:7041",
            },
            clear=True,
        ):
            client = backend_stack.RpcClient(
                env_prefix="ELEMENTS",
                default_url="http://elementsd:7041",
            )

        self.assertEqual(client.url, "http://elements.example:7041")
        self.assertEqual(client.user, "fallback-user")
        self.assertEqual(client.password, "fallback-pass")

    def test_liquid_electrum_calls_delegate_to_real_index(self) -> None:
        index = SimpleNamespace(
            rpc=SimpleNamespace(call=lambda _method: {"chain": "elementsregtest"}),
            tip_height=lambda: 42,
            block_header=lambda height: f"header-{height}",
            history=lambda scripthash: [{"tx_hash": f"{scripthash}-tx", "height": 42}],
            utxos=lambda scripthash: [
                {"tx_hash": f"{scripthash}-tx", "tx_pos": 0, "value": 0}
            ],
            raw_hex=lambda txid: f"raw-{txid}",
        )
        handler = object.__new__(backend_stack.ElectrumHandler)
        handler.server = SimpleNamespace(
            chain="liquid",
            network="elementsregtest",
            service_name="liquid-electrum-regtest",
            index=index,
        )

        self.assertEqual(handler._call("server.version", []), ["Kassiber regtest backend", "1.4"])
        self.assertEqual(
            handler._call("blockchain.headers.subscribe", []),
            {"height": 42, "hex": "header-42"},
        )
        self.assertEqual(
            handler._call("blockchain.scripthash.get_history", ["abcd"]),
            [{"tx_hash": "abcd-tx", "height": 42}],
        )
        self.assertEqual(
            handler._call("blockchain.scripthash.listunspent", ["abcd"]),
            [{"tx_hash": "abcd-tx", "tx_pos": 0, "value": 0}],
        )
        self.assertEqual(handler._call("blockchain.transaction.get", ["txid"]), "raw-txid")
        self.assertEqual(handler._call("blockchain.block.header", [41]), "header-41")

    def test_liquid_api_scripthash_stats_delegate_to_real_index(self) -> None:
        captured: list[object] = []

        class CapturingHandler(backend_stack.ApiHandler):
            def __init__(self) -> None:
                self.server = SimpleNamespace(
                    chain="liquid",
                    index=SimpleNamespace(stats=lambda scripthash: {"seen": scripthash}),
                )

            def _json(self, value) -> None:
                captured.append(value)

            def _error(self, code, message) -> None:
                raise AssertionError(f"unexpected error {code}: {message}")

        CapturingHandler()._scripthash("/api/scripthash/deadbeef")

        self.assertEqual(captured, [{"seen": "deadbeef"}])

    def test_index_utxos_include_electrum_and_esplora_keys(self) -> None:
        script_hex = "0014" + "11" * 20

        class FakeRpc:
            def call(self, method, params=None):
                params = params or []
                if method == "getblockcount":
                    return 1
                if method == "getblockhash":
                    return f"block-{params[0]}"
                if method == "getrawmempool":
                    return []
                if method == "getblock":
                    height = int(str(params[0]).split("-", 1)[1])
                    return {
                        "tx": [
                            {
                                "txid": f"tx-{height}",
                                "blockhash": params[0],
                                "time": 1_700_000_000 + height,
                                "vout": [
                                    {
                                        "n": 0,
                                        "value": 0.00010000,
                                        "scriptPubKey": {
                                            "hex": script_hex,
                                            "type": "witness_v0_keyhash",
                                        },
                                    }
                                ],
                            }
                        ]
                    }
                raise AssertionError(f"unexpected RPC call: {method} {params}")

        index = backend_stack.BitcoinIndex(FakeRpc())
        rows = index.utxos(backend_stack.electrum_scripthash(script_hex))

        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[1]["tx_hash"], "tx-1")
        self.assertEqual(rows[1]["tx_pos"], 0)
        self.assertEqual(rows[1]["height"], 1)
        self.assertEqual(rows[1]["txid"], "tx-1")
        self.assertEqual(rows[1]["vout"], 0)

    def test_index_history_includes_spends_by_prevout_script(self) -> None:
        script_hex = "0014" + "22" * 20

        class FakeRpc:
            def call(self, method, params=None):
                params = params or []
                if method == "getblockcount":
                    return 2
                if method == "getblockhash":
                    return f"block-{params[0]}"
                if method == "getrawmempool":
                    return []
                if method == "getrawtransaction" and params[0] == "funding-tx":
                    return {
                        "txid": "funding-tx",
                        "vout": [
                            {
                                "n": 0,
                                "value": 0.001,
                                "scriptPubKey": {
                                    "hex": script_hex,
                                    "type": "witness_v0_keyhash",
                                },
                            }
                        ],
                    }
                if method == "getblock":
                    height = int(str(params[0]).split("-", 1)[1])
                    if height == 0:
                        return {"tx": []}
                    if height == 1:
                        return {
                            "tx": [
                                {
                                    "txid": "funding-tx",
                                    "blockhash": params[0],
                                    "time": 1_700_000_000,
                                    "vout": [
                                        {
                                            "n": 0,
                                            "value": 0.001,
                                            "scriptPubKey": {
                                                "hex": script_hex,
                                                "type": "witness_v0_keyhash",
                                            },
                                        }
                                    ],
                                }
                            ]
                        }
                    return {
                        "tx": [
                            {
                                "txid": "spend-tx",
                                "blockhash": params[0],
                                "time": 1_700_000_600,
                                "vin": [{"txid": "funding-tx", "vout": 0}],
                                "vout": [
                                    {
                                        "n": 0,
                                        "value": 0.0009,
                                        "scriptPubKey": {
                                            "hex": "0014" + "33" * 20,
                                            "type": "witness_v0_keyhash",
                                        },
                                    }
                                ],
                            }
                        ]
                    }
                raise AssertionError(f"unexpected RPC call: {method} {params}")

        index = backend_stack.BitcoinIndex(FakeRpc())
        rows = index.history(backend_stack.electrum_scripthash(script_hex))

        self.assertEqual(rows, [{"tx_hash": "funding-tx", "height": 1}, {"tx_hash": "spend-tx", "height": 2}])


if __name__ == "__main__":
    unittest.main()
