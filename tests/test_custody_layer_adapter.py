from __future__ import annotations

import unittest

from kassiber.core.custody_layer_adapter import (
    CustodyLayerEvent,
    build_layer_quantity_input,
    layer_evidence_from_observation,
)


class _FutureBitcoinLayerAdapter:
    def custody_events(self):
        common = {
            "layer": "future-layer",
            "network": "main",
            "native_namespace": "future-layer",
            "native_event_id": "round-42",
            "asset": "BTC",
            "exposure": "bitcoin",
            "occurred_at": "2030-01-02T03:04:05Z",
            "custody_state": "owned",
            "finality_state": "final",
            "exit_state": "cooperative",
            "parent_event_ids": ("funding-41",),
            "spent_event_ids": ("vtxo-7",),
            "evidence_provenance": (("adapter", "future-layer-v1"),),
        }
        return (
            CustodyLayerEvent(
                **common,
                wallet_id="old-vault",
                direction="outbound",
                amount_msat=1_000_000,
                fee_msat=1_000,
            ),
            CustodyLayerEvent(
                **common,
                wallet_id="new-vault",
                direction="inbound",
                amount_msat=999_000,
                fee_msat=0,
            ),
        )


class FutureCustodyLayerContractTests(unittest.TestCase):
    def test_new_layer_reaches_quantity_boundary_without_tax_types(self) -> None:
        quantity_input = build_layer_quantity_input(_FutureBitcoinLayerAdapter())

        self.assertFalse(quantity_input.rejected_events)
        self.assertEqual(len(quantity_input.events), 1)
        self.assertEqual(len(quantity_input.observations), 2)
        outbound = next(
            item for item in quantity_input.observations if item.direction == "outbound"
        )
        self.assertEqual(outbound.event_key.native_namespace, "future-layer")
        self.assertEqual(outbound.event_key.native_event_id, "round-42")
        self.assertEqual(outbound.fee_msat, 1_000)

        evidence = layer_evidence_from_observation(outbound.evidence_payload_json)
        self.assertEqual(evidence["exposure"], "bitcoin")
        self.assertEqual(evidence["parent_event_ids"], ["funding-41"])
        self.assertEqual(evidence["spent_event_ids"], ["vtxo-7"])
        self.assertEqual(evidence["custody_state"], "owned")
        self.assertEqual(evidence["finality_state"], "final")
        self.assertEqual(evidence["exit_state"], "cooperative")
        self.assertEqual(evidence["provenance"], {"adapter": "future-layer-v1"})


if __name__ == "__main__":
    unittest.main()
