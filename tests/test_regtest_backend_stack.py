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

    def test_esplora_scripthash_preserves_electrum_index_order(self) -> None:
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

    def test_esplora_address_routes_resolve_script_through_chain_rpc(self) -> None:
        captured: list[object] = []
        script = "0014" + "11" * 20
        expected = backend_stack.electrum_scripthash(script)

        class CapturingHandler(backend_stack.ApiHandler):
            def __init__(self) -> None:
                self.server = SimpleNamespace(
                    index=SimpleNamespace(
                        rpc=SimpleNamespace(
                            call=lambda method, params: {
                                "isvalid": method == "validateaddress" and params == ["ert1qtest"],
                                "scriptPubKey": script,
                            }
                        ),
                        stats=lambda scripthash: {"seen": scripthash},
                    )
                )

            def _json(self, value) -> None:
                captured.append(value)

            def _error(self, code, message) -> None:
                raise AssertionError(f"unexpected error {code}: {message}")

        CapturingHandler()._address("/api/address/ert1qtest")
        self.assertEqual(captured, [{"seen": expected}])

    def test_lwk_custom_elements_hrp_is_translated_for_elements_core(self) -> None:
        payload = [0, 1, 2, 3, 4]
        values = backend_stack._blech32_hrp_expand("el") + payload + [0] * 12
        polymod = backend_stack._blech32_polymod(values) ^ 1
        checksum = [(polymod >> (5 * (11 - index))) & 31 for index in range(12)]
        address = "el1" + "".join(
            backend_stack.BLECH32_CHARSET[value] for value in payload + checksum
        )

        translated = backend_stack._replace_blech32_hrp(address, "ert")

        self.assertTrue(translated.startswith("ert1"))
        encoded = [
            backend_stack.BLECH32_CHARSET.index(char)
            for char in translated.split("1", 1)[1]
        ]
        self.assertEqual(encoded[:-12], payload)
        self.assertEqual(
            backend_stack._blech32_polymod(
                backend_stack._blech32_hrp_expand("ert") + encoded
            ),
            1,
        )

    def test_liquid_esplora_payload_omits_custom_network_address(self) -> None:
        index = backend_stack.BitcoinIndex(SimpleNamespace(), chain="liquid")
        payload = index._to_esplora(
            {
                "txid": "11" * 32,
                "vout": [
                    {
                        "n": 0,
                        "value": "1.0",
                        "scriptPubKey": {
                            "hex": "0014" + "22" * 20,
                            "address": "ert1-invalid-to-lwk",
                        },
                    }
                ],
            }
        )

        self.assertNotIn("scriptpubkey_address", payload["vout"][0])

    def test_esplora_tip_hash_uses_the_current_chain_tip(self) -> None:
        captured: list[str] = []

        class CapturingHandler(backend_stack.ApiHandler):
            def __init__(self) -> None:
                self.path = "/api/blocks/tip/hash"
                self.server = SimpleNamespace(
                    index=SimpleNamespace(
                        tip_height=lambda: 42,
                        rpc=SimpleNamespace(
                            call=lambda method, params: (
                                "tip-42"
                                if method == "getblockhash" and params == [42]
                                else None
                            )
                        ),
                    )
                )

            def _text(self, value) -> None:
                captured.append(value)

            def _error(self, code, message) -> None:
                raise AssertionError(f"unexpected error {code}: {message}")

        CapturingHandler().do_GET()
        self.assertEqual(captured, ["tip-42"])

    def test_esplora_raw_transaction_is_binary_not_hex_text(self) -> None:
        captured: list[bytes] = []

        class CapturingHandler(backend_stack.ApiHandler):
            def __init__(self) -> None:
                self.server = SimpleNamespace(
                    index=SimpleNamespace(raw_hex=lambda txid: "0102" if txid == "tx" else "")
                )

            def _binary(self, value) -> None:
                captured.append(value)

            def _error(self, code, message) -> None:
                raise AssertionError(f"unexpected error {code}: {message}")

        CapturingHandler()._tx_raw("tx")
        self.assertEqual(captured, [b"\x01\x02"])

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

    def test_index_history_includes_no_change_spend_via_prevout_script(self) -> None:
        owned_script = "0014" + "11" * 20
        external_script = "0014" + "22" * 20
        index = backend_stack.BitcoinIndex(SimpleNamespace())
        history: dict[str, list[dict[str, object]]] = {}
        spent: set[tuple[str, int]] = set()
        transaction = {
            "txid": "spend",
            "vin": [{"txid": "funding", "vout": 2}],
            "vout": [
                {
                    "n": 0,
                    "value": 0.00009000,
                    "scriptPubKey": {"hex": external_script},
                }
            ],
        }

        previous = {
            "txid": "funding",
            "vout": [
                {"n": 0, "value": 0.0, "scriptPubKey": {"hex": external_script}},
                {"n": 1, "value": 0.0, "scriptPubKey": {"hex": external_script}},
                {"n": 2, "value": 0.0001, "scriptPubKey": {"hex": owned_script}},
            ],
        }
        with patch.object(
            index,
            "_prevout",
            side_effect=AssertionError("known prevout used RPC fallback"),
        ) as fallback:
            index._index_tx(
                history,
                spent,
                transaction,
                0,
                known_txs={"funding": previous},
            )

        self.assertEqual(spent, {("funding", 2)})
        fallback.assert_not_called()
        self.assertEqual(
            history[backend_stack.electrum_scripthash(owned_script)],
            [{"tx_hash": "spend", "height": 0}],
        )


if __name__ == "__main__":
    unittest.main()
