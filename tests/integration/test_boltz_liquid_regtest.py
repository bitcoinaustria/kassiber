from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from tests.integration import boltz_liquid_regtest
from tests.integration.env import no_egress_guard, skip_unless_env


def _txid(label: str) -> str:
    return hashlib.sha256(label.encode("utf-8")).hexdigest()


LIQUID_POLICY_ASSET_ID = _txid("elements regtest policy asset")


class BoltzLiquidRegtestTest(unittest.TestCase):
    def test_demo_boltz_bridge_metadata_is_self_contained(self):
        scenario = boltz_liquid_regtest.load_demo_scenario_metadata()
        bridges = boltz_liquid_regtest.boltz_bridge_specs(scenario)

        self.assertEqual(len(bridges), 1)
        self.assertEqual(
            {
                (
                    bridge["boltz_flow"],
                    bridge["boltz_api"],
                    bridge["boltz_from"],
                    bridge["boltz_to"],
                )
                for bridge in bridges
            },
            {("chain-swap", "/v2/swap/chain", "BTC", "L-BTC")},
        )

    @skip_unless_env("KASSIBER_BOLTZ_REGTEST", "local Boltz regtest stack is opt-in")
    def test_live_boltz_liquid_pairs_cover_demo_bridge(self):
        with no_egress_guard(enabled=True):
            probe = boltz_liquid_regtest.probe_boltz_liquid()

        self.assertIn("version", probe)
        self.assertGreaterEqual(int(probe["heights"]["BTC"]), 0)
        self.assertGreaterEqual(int(probe["heights"]["L-BTC"]), 0)
        self.assertIn("hash", probe["pairs"]["bitcoin_to_liquid"])

        covered = boltz_liquid_regtest.verify_demo_boltz_coverage(
            probe,
            boltz_liquid_regtest.load_demo_scenario_metadata(),
        )
        self.assertEqual(
            {(row["flow"], row["from"], row["to"]) for row in covered},
            {("chain-swap", "BTC", "L-BTC")},
        )

    def test_accounting_builder_covers_executed_swap_and_payment_only(self):
        payment = {
            "txid": "11" * 32,
            "amount_sats": 77777,
            "amount": "0.00077777",
            "asset": "LBTC",
            "asset_id": LIQUID_POLICY_ASSET_ID,
        }
        swap = {
            "id": "unit-submarine",
            "payment_hash": "ab" * 32,
            "invoice_sats": 100000,
            "expected_amount_sats": 101000,
            "expected_amount": "0.00101000",
            "lockup_txid": "22" * 32,
            "asset_id": LIQUID_POLICY_ASSET_ID,
            "status": "invoice.paid",
        }

        with tempfile.TemporaryDirectory(prefix="kassiber-boltz-accounting-") as tmp:
            accounting = boltz_liquid_regtest._build_accounting_book(  # noqa: SLF001
                Path(tmp) / "data",
                payment=payment,
                swap=swap,
            )

        self.assertEqual(accounting["imports"]["liquid_rows"], 2)
        self.assertEqual(accounting["imports"]["lightning_rows"], 1)
        self.assertEqual(accounting["imports"]["boltz_v2_evidence_rows"], 0)
        self.assertEqual(accounting["boltz_v2_pairs"]["count"], 0)
        self.assertEqual(accounting["candidate"]["confidence"], "strong")
        self.assertEqual(accounting["candidate"]["method"], "payment_hash")
        self.assertEqual(accounting["pair"]["kind"], "submarine-swap")
        self.assertFalse(accounting["plain_payment"]["paired"])
        self.assertEqual(
            accounting["lightning_settlement"],
            {
                "kind": "lnd_invoice",
                "payment_hash": swap["payment_hash"],
                "payment_hash_source": "lnd",
                "network": "regtest",
                "import_source": "lnd",
            },
        )

    def test_accounting_builder_pairs_real_boltz_v2_evidence(self):
        payment = {
            "txid": _txid("plain liquid payment"),
            "amount_sats": 77777,
            "amount": "0.00077777",
            "asset": "LBTC",
            "asset_id": LIQUID_POLICY_ASSET_ID,
        }
        swap = {
            "id": "unit-submarine",
            "payment_hash": "ab" * 32,
            "invoice_sats": 100000,
            "expected_amount_sats": 101000,
            "expected_amount": "0.00101000",
            "lockup_txid": _txid("submarine lockup"),
            "asset_id": LIQUID_POLICY_ASSET_ID,
            "status": "invoice.paid",
        }
        evidence_payload = {
            "swaps": [
                {
                    "provider": "boltz",
                    "id": "real-v2-chain-evidence",
                    "flow": "chain",
                    "status": "completed",
                    "version": "2",
                    "taproot": True,
                    "cooperative": True,
                    "spend_path": "key",
                    "out": {
                        "txid": _txid("real boltz chain send"),
                        "occurred_at": "2026-07-02T11:00:00Z",
                        "asset": "BTC",
                        "chain": "bitcoin",
                        "network": "regtest",
                        "amount": "0.01000000",
                        "fee": "0.00000500",
                    },
                    "in": {
                        "txid": _txid("real boltz chain receive"),
                        "occurred_at": "2026-07-02T11:04:00Z",
                        "asset": "LBTC",
                        "asset_id": LIQUID_POLICY_ASSET_ID,
                        "chain": "liquid",
                        "network": "elementsregtest",
                        "amount": "0.00990000",
                    },
                }
            ]
        }

        with tempfile.TemporaryDirectory(prefix="kassiber-boltz-accounting-") as tmp:
            root = Path(tmp)
            evidence_path = root / "boltz-v2-evidence.json"
            evidence_path.write_text(json.dumps(evidence_payload), encoding="utf-8")
            accounting = boltz_liquid_regtest._build_accounting_book(  # noqa: SLF001
                root / "data",
                payment=payment,
                swap=swap,
                boltz_v2_evidence=evidence_path,
            )

        self.assertEqual(accounting["imports"]["boltz_v2_evidence_rows"], 2)
        self.assertEqual(accounting["boltz_v2_pairs"]["count"], 1)
        self.assertEqual(accounting["boltz_v2_pairs"]["kinds"], ["chain-swap"])

    def test_real_boltz_v2_evidence_rejects_placeholder_route_ids(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-boltz-accounting-") as tmp:
            evidence_path = Path(tmp) / "boltz-v2-evidence.json"
            evidence_path.write_text(
                json.dumps(
                    {
                        "swaps": [
                            {
                                "provider": "boltz",
                                "id": "placeholder-chain",
                                "flow": "chain",
                                "out": {
                                    "txid": "aa" * 32,
                                    "occurred_at": "2026-07-02T11:00:00Z",
                                    "asset": "BTC",
                                    "amount": "0.01000000",
                                },
                                "in": {
                                    "txid": _txid("real receive"),
                                    "occurred_at": "2026-07-02T11:04:00Z",
                                    "asset": "LBTC",
                                    "amount": "0.00990000",
                                },
                            }
                        ]
                    }
                ),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(boltz_liquid_regtest.BoltzProbeError, "placeholder-looking"):
                boltz_liquid_regtest._boltz_v2_evidence_rows(evidence_path)  # noqa: SLF001

    def test_liquid_leg_requires_consensus_asset_identity(self):
        with self.assertRaisesRegex(
            boltz_liquid_regtest.BoltzProbeError,
            "missing asset_id",
        ):
            boltz_liquid_regtest._boltz_leg_identity(  # noqa: SLF001
                {
                    "chain": "liquid",
                    "network": "elementsregtest",
                    "asset": "LBTC",
                },
                external_id=_txid("liquid leg without asset id"),
                asset="LBTC",
                context="test leg",
            )

    def test_provider_evidence_with_lightning_leg_stays_strong(self):
        payload = {
            "swaps": [
                {
                    "provider": "boltz",
                    "id": "reverse-with-lightning-leg",
                    "flow": "reverse",
                    "status": "completed",
                    "out": {
                        "payment_hash": _txid("reverse lightning payment"),
                        "occurred_at": "2026-07-02T12:00:00Z",
                        "asset": "BTC",
                        "chain": "lightning",
                        "network": "regtest",
                        "amount": "0.01000000",
                    },
                    "in": {
                        "txid": _txid("reverse liquid claim"),
                        "occurred_at": "2026-07-02T12:04:00Z",
                        "asset": "LBTC",
                        "asset_id": LIQUID_POLICY_ASSET_ID,
                        "chain": "liquid",
                        "network": "elementsregtest",
                        "amount": "0.00990000",
                    },
                }
            ]
        }
        with tempfile.TemporaryDirectory(prefix="kassiber-boltz-evidence-") as tmp:
            path = Path(tmp) / "evidence.json"
            path.write_text(json.dumps(payload), encoding="utf-8")
            rows = boltz_liquid_regtest._boltz_v2_evidence_rows(path)  # noqa: SLF001

        expected = rows["expected"]["reverse-submarine-swap"]
        self.assertEqual("strong", expected["confidence"])
        self.assertEqual(
            "lightning", rows["out_rows"][0]["raw_json"]["chain"]
        )
        self.assertEqual(
            LIQUID_POLICY_ASSET_ID,
            rows["in_rows"][0]["raw_json"]["component"]["asset_id"],
        )

    @skip_unless_env("KASSIBER_BOLTZ_REGTEST", "local Boltz regtest stack is opt-in")
    def test_live_boltz_liquid_execution_covers_swap_and_payment_accounting(self):
        with no_egress_guard(enabled=True):
            summary = boltz_liquid_regtest.run_boltz_liquid_scenario()

        payment = summary["executed"]["liquid_payment"]
        swap = summary["executed"]["liquid_submarine_swap"]
        accounting = summary["accounting"]

        self.assertEqual(payment["asset"], "LBTC")
        self.assertRegex(payment["txid"], r"^[0-9a-f]{64}$")
        self.assertEqual(swap["payment_hash"], accounting["swap_lockup"]["payment_hash"])
        self.assertEqual(accounting["swap_lockup"]["payment_hash_source"], "boltz-regtest")
        self.assertEqual(accounting["plain_payment"]["asset"], "LBTC")
        self.assertEqual(accounting["plain_payment"]["direction"], "outbound")
        self.assertFalse(accounting["plain_payment"]["paired"])
        self.assertEqual(accounting["boltz_v2_pairs"]["count"], 0)

        candidate = accounting["candidate"]
        self.assertEqual(candidate["confidence"], "strong")
        self.assertEqual(candidate["method"], "payment_hash")
        self.assertEqual(candidate["out_asset"], "LBTC")
        self.assertEqual(candidate["in_asset"], "BTC")
        self.assertEqual(candidate["out_wallet_kind"], "custom")
        self.assertEqual(candidate["in_wallet_kind"], "lnd")
        self.assertEqual(candidate["default_kind"], "submarine-swap")
        self.assertEqual(candidate["candidate_type"], "transfer")
        self.assertEqual(
            accounting["lightning_settlement"]["payment_hash_source"], "lnd"
        )
        self.assertEqual(
            accounting["lightning_settlement"]["import_source"], "lnd"
        )
        self.assertEqual(accounting["lightning_settlement"]["network"], "regtest")

        pair = accounting["pair"]
        self.assertEqual(pair["kind"], "submarine-swap")
        self.assertEqual(pair["out"]["external_id"], swap["lockup_txid"])
        self.assertEqual(pair["out"]["asset"], "LBTC")
        self.assertEqual(pair["in"]["asset"], "BTC")


if __name__ == "__main__":
    unittest.main()
