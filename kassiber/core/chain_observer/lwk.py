"""LWK-backed, observation-only Liquid chain observer."""

from __future__ import annotations

import os
import re
import json
from decimal import Decimal
from typing import Any, Mapping

from ...backends import backend_batch_size, backend_timeout, backend_value
from ...egress_ledger import endpoint_from_url, get_egress_ledger
from ...envelope import json_ready
from ...errors import AppError
from ...redaction import redact_operational_text, redact_secret_text
from ...time_utils import UNKNOWN_OCCURRED_AT, timestamp_to_iso
from ...proxy import is_onion_endpoint
from ...wallet_descriptors import liquid_asset_code
from ..sync import emit_sync_progress
from .contract import ChainFacts, ObserverApplication, ObserverPrepareRequest
from .identity import ObserverIdentity
from .lwk_persistence import SqlCipherForeignStore, require_lwk
from .store import CoveragePoint, StoredObserverState


LWK_OBSERVER_STATE_VERSION = 1


def _truthy_env(name: str) -> bool:
    return str(os.environ.get(name) or "").strip().lower() in {"1", "true", "yes", "on"}


def lwk_compatibility_reason(backend: Mapping[str, Any], sync_state: Any) -> str | None:
    """Preflight configurations the Python binding cannot safely represent."""

    if getattr(sync_state, "chain", None) != "liquid":
        return "non_liquid_chain"
    plan = getattr(sync_state, "descriptor_plan", None)
    if plan is None:
        return "address_list"
    kind = str(backend.get("kind") or "").strip().lower()
    if kind not in {"esplora", "electrum"}:
        return "backend_kind"
    endpoint = str(backend.get("url") or "")
    proxy = backend_value(backend, "tor_proxy", "proxy")
    if proxy or is_onion_endpoint(endpoint):
        return "proxy_transport"
    if backend_value(backend, "certificate"):
        return "custom_ca"
    if kind == "esplora" and (
        backend_value(backend, "auth_header") or backend_value(backend, "token")
    ):
        return "custom_http_auth"
    if kind == "electrum" and backend_value(backend, "timeout") is not None:
        try:
            if int(backend_value(backend, "timeout")) != 30:
                return "custom_timeout"
        except (TypeError, ValueError):
            return "custom_timeout"
    branches = tuple(getattr(plan, "branches", ()))
    if len(branches) not in {1, 2}:
        return "noncanonical_multipath"
    if len(branches) == 2:
        if tuple(getattr(branch, "selector", None) for branch in branches) != (0, 1):
            return "separate_change_descriptor"
        if str(branches[0].descriptor) != str(branches[1].descriptor):
            return "separate_change_descriptor"
    try:
        lwk_descriptor_for_plan(plan)
    except Exception:
        return "descriptor_unsupported"
    return None


def lwk_network(network: str, policy_asset_id: str):
    lwk = require_lwk()
    key = str(network).strip().lower()
    if key == "liquidv1":
        return lwk.Network.mainnet()
    if key == "liquidtestnet":
        return lwk.Network.testnet()
    if key == "elementsregtest":
        return lwk.Network.regtest(str(policy_asset_id))
    raise AppError("Unsupported LWK network", code="observer_capability_unsupported")


def lwk_descriptor_for_plan(plan: Any):
    """Translate the executable embit CT plan into LWK's CT spelling."""

    lwk = require_lwk()
    descriptor = plan.branches[0].descriptor
    text = str(descriptor)
    if not text.startswith("blinded("):
        raise AppError("LWK requires a confidential descriptor", code="observer_capability_unsupported")
    blinding = descriptor.blinding_key
    if getattr(blinding, "slip77", False):
        key = bytes(descriptor.master_blinding_key).hex()
        text = re.sub(r"^blinded\(slip77\([^)]*\),", f"ct(slip77({key}),", text)
    else:
        key = getattr(blinding, "key", None)
        if key is None or not getattr(key, "is_private", False):
            raise AppError("LWK requires private view material", code="observer_capability_unsupported")
        text = re.sub(r"^blinded\([^,]+,", f"ct({key},", text)
    replacements = (
        (r"(?<![a-z])wpkh\(", "elwpkh("),
        (r"(?<![a-z])wsh\(", "elwsh("),
        (r"(?<![a-z])tr\(", "eltr("),
        (r"(?<![a-z])sh\(", "elsh("),
    )
    for pattern, target in replacements:
        text = re.sub(pattern, target, text)
    return lwk.WolletDescriptor(text)


def _btc(sats: int) -> Decimal:
    return Decimal(int(sats)) / Decimal(100_000_000)


def _fee_sats_by_asset(outputs: list[Any]) -> dict[str, int]:
    """Read Elements' explicit fee outputs through LWK transaction objects."""

    fees: dict[str, int] = {}
    for output in outputs:
        if not output.is_fee() or output.value() is None or output.asset() is None:
            continue
        asset_id = str(output.asset())
        fees[asset_id] = fees.get(asset_id, 0) + int(output.value())
    return fees


class LwkObserver:
    def __init__(
        self,
        *,
        identity: ObserverIdentity,
        backend: Mapping[str, Any],
        descriptor_plan: Any,
        policy_asset_id: str,
        stored_values: Mapping[str, bytes],
    ):
        self.identity = identity
        self.backend = dict(backend)
        self.plan = descriptor_plan
        self.policy_asset_id = str(policy_asset_id)
        self.store = SqlCipherForeignStore(identity, stored_values)
        self._wallet = None
        self._store_link = None

    def _client(self, network):
        lwk = require_lwk()
        endpoint = str(self.backend.get("url") or "").strip()
        if not endpoint:
            raise AppError("LWK observer backend is missing its endpoint", code="validation")
        if _truthy_env("KASSIBER_NO_EGRESS"):
            raise AppError(
                "Outbound chain observation is disabled by KASSIBER_NO_EGRESS",
                code="network_egress_disabled",
                retryable=False,
            )
        host, port, scheme = endpoint_from_url(endpoint)
        get_egress_ledger().record(
            subsystem="sync",
            host=host,
            port=port,
            scheme=scheme,
            operation=f"lwk.{self.backend['kind']}.connect",
            via_proxy=False,
        )
        kind = str(self.backend["kind"]).lower()
        if kind == "esplora":
            builder = lwk.EsploraClientBuilder(
                base_url=endpoint,
                network=network,
                concurrency=max(1, min(8, backend_batch_size(self.backend))),
                timeout=max(1, min(255, backend_timeout(self.backend))),
            )
            return lwk.EsploraClient.from_builder(builder)
        if kind == "electrum":
            return lwk.ElectrumClient.from_url(endpoint)
        raise AppError("Unsupported LWK observer backend", code="observer_capability_unsupported")

    @staticmethod
    def _owned_payload(output: Any) -> dict[str, Any]:
        secrets = output.unblinded()
        return {
            "scriptpubkey": bytes(output.script_pubkey().to_bytes()).hex(),
            "value_sats": int(secrets.value()),
            "asset_id": str(secrets.asset()),
            "role": "owned",
        }

    def _records(self, wallet_tx: Any) -> list[Mapping[str, Any]]:
        from ..sync_backends import (
            _extract_refund_funding_outpoint,
            _extract_unique_claim_payment_hash_outpoint,
            _liquid_witness_items,
            _payment_hash_fields,
            _swap_refund_fields,
            liquid_input_txid,
            liquid_input_vout,
        )
        from ...wallet_descriptors import decode_liquid_transaction

        tx = wallet_tx.tx()
        txid = str(wallet_tx.txid())
        owned_inputs = list(wallet_tx.inputs())
        owned_outputs = list(wallet_tx.outputs())
        raw_inputs = list(tx.inputs())
        raw_outputs = list(tx.outputs())
        fee_sats_by_asset = _fee_sats_by_asset(raw_outputs)
        stored_vin = []
        for index, txin in enumerate(raw_inputs):
            outpoint = txin.outpoint()
            item = {"txid": str(outpoint.txid()), "vout": int(outpoint.vout())}
            owned = owned_inputs[index] if index < len(owned_inputs) else None
            if owned is not None:
                item["prevout"] = self._owned_payload(owned)
            stored_vin.append(item)
        stored_vout = []
        for index, txout in enumerate(raw_outputs):
            item = {"n": index, "scriptpubkey": bytes(txout.script_pubkey().to_bytes()).hex()}
            owned = owned_outputs[index] if index < len(owned_outputs) else None
            if owned is not None:
                item.update(self._owned_payload(owned))
            elif txout.is_fee():
                item.update(
                    {
                        "value_sats": int(txout.value() or 0),
                        "asset_id": str(txout.asset()),
                        "role": "fee",
                    }
                )
            elif txout.value() is not None and txout.asset() is not None:
                item.update(
                    {"value_sats": int(txout.value()), "asset_id": str(txout.asset()), "role": "external"}
                )
            stored_vout.append(item)
        decoded = decode_liquid_transaction(tx.to_bytes().hex())
        claim = _extract_unique_claim_payment_hash_outpoint(
            decoded.vin, _liquid_witness_items,
            prev_txid_fn=liquid_input_txid, prev_vout_fn=liquid_input_vout,
        )
        refund = _extract_refund_funding_outpoint(
            decoded.vin, _liquid_witness_items,
            prev_txid_fn=liquid_input_txid, prev_vout_fn=liquid_input_vout,
        )
        timestamp = wallet_tx.timestamp()
        occurred_at = timestamp_to_iso(timestamp) if timestamp is not None else UNKNOWN_OCCURRED_AT
        confirmed_at = occurred_at if timestamp is not None else None
        owns_input = any(item is not None for item in owned_inputs)
        records = []
        for asset_id, signed_value in sorted(wallet_tx.balance().items(), key=lambda item: str(item[0])):
            asset_id = str(asset_id)
            net = int(signed_value)
            fee = fee_sats_by_asset.get(asset_id, 0) if owns_input and net <= 0 else 0
            if net == 0 and fee == 0:
                continue
            if net > 0:
                direction, amount, record_fee, kind = "inbound", _btc(net), Decimal(0), "deposit"
            else:
                direction = "outbound"
                amount = _btc(max(0, abs(net) - fee))
                record_fee = _btc(fee)
                kind = "withdrawal" if amount > 0 else "fee"
            asset = liquid_asset_code(asset_id, self.policy_asset_id)
            records.append(
                {
                    "txid": txid,
                    "occurred_at": occurred_at,
                    "confirmed_at": confirmed_at,
                    "direction": direction,
                    "asset": asset,
                    "amount": str(amount),
                    "fee": str(record_fee),
                    "fiat_rate": None,
                    "fiat_value": None,
                    "kind": kind,
                    "description": f"Synced from {self.backend['name']}",
                    "counterparty": None,
                    "raw_json": json.dumps(
                        json_ready(
                            {
                                "txid": txid,
                                "chain": "liquid",
                                "network": self.identity.network,
                                "observer": "lwk",
                                "ownership_graph_version": 1,
                                "vin": stored_vin,
                                "vout": stored_vout,
                                "component": {
                                    "asset_id": asset_id,
                                    "asset": asset,
                                    "net_sats": net,
                                    "fee_sats": fee,
                                },
                            }
                        ),
                        sort_keys=True,
                    ),
                    **_payment_hash_fields(claim),
                    **_swap_refund_fields(*(refund or (None, None))),
                }
            )
        return records

    def _facts(self, wollet: Any, tip: Any, prior_state: StoredObserverState | None) -> ChainFacts:
        transactions = list(wollet.transactions())
        records = [record for wallet_tx in transactions for record in self._records(wallet_tx)]
        current_txids = {str(wallet_tx.txid()) for wallet_tx in transactions}
        previous_txids = set(prior_state.payload.get("canonical_txids") or ()) if prior_state else set()
        tip_height = int(tip.height())
        outputs = []
        highest: dict[str, int] = {}
        for output in wollet.txos():
            branch_key = "change" if str(output.ext_int()).lower().endswith("internal") else "receive"
            highest[branch_key] = max(highest.get(branch_key, -1), int(output.wildcard_index()))
        for output in wollet.utxos():
            branch_key = "change" if str(output.ext_int()).lower().endswith("internal") else "receive"
            index = int(output.wildcard_index())
            secrets = output.unblinded()
            height = output.height()
            outpoint = output.outpoint()
            outputs.append(
                {
                    "txid": str(outpoint.txid()), "vout": int(outpoint.vout()),
                    "asset": liquid_asset_code(str(secrets.asset()), self.policy_asset_id),
                    "amount_sats": int(secrets.value()),
                    "confirmation_status": "confirmed" if height is not None else "mempool",
                    "confirmations": max(0, tip_height - int(height) + 1) if height is not None else 0,
                    "block_height": int(height) if height is not None else None,
                    "block_time": None,
                    "chain": "liquid", "network": self.identity.network,
                    "branch_index": 1 if branch_key == "change" else 0,
                    "branch_label": branch_key,
                    "address_index": index,
                    "address": str(output.address()),
                    "script_pubkey": bytes(output.script_pubkey().to_bytes()).hex(),
                    "derivation_path": None, "derivation_paths": [], "key_origins": [],
                    "raw": {"source": "lwk_wollet", "confidential": True},
                }
            )
        coverage = tuple(
            CoveragePoint(
                branch_key=key,
                scanned_to=max(1, int(self.plan.gap_limit), highest.get(key, -1) + int(self.plan.gap_limit) + 1),
                highest_used=highest.get(key),
                details={"observer": "lwk"},
            )
            for key in self.identity.branch_keys
        )
        checkpoint = {
            "observer": "lwk", "tip_height": tip_height, "tip_hash": str(tip.block_hash()),
            "highest_used": {
                str(1 if key == "change" else 0): value for key, value in sorted(highest.items())
            },
            "canonical_txids": sorted(current_txids),
        }
        return ChainFacts(
            transaction_records=tuple(records),
            retracted_external_ids=tuple(sorted(previous_txids - current_txids)),
            outputs=tuple(outputs), coverage=coverage, freshness_checkpoint=checkpoint,
        )

    def prepare(self, request: ObserverPrepareRequest, prior_state: StoredObserverState | None) -> Mapping[str, Any]:
        try:
            if prior_state is not None and prior_state.payload.get("schema_version") != LWK_OBSERVER_STATE_VERSION:
                raise AppError("Stored LWK observer state must be rebuilt", code="observer_state_rebuild_required", retryable=False)
            lwk = require_lwk()
            network = lwk_network(self.identity.network, self.policy_asset_id)
            self._store_link = lwk.ForeignStoreLink(self.store)
            self._wallet = lwk.Wollet.with_custom_store(network, lwk_descriptor_for_plan(self.plan), self._store_link)
            client = self._client(network)
            emit_sync_progress({"phase": "backend_fetch", "observer": "lwk", "status": "scanning"})
            scan_to = max(0, int(self.plan.gap_limit) - 1)
            update = client.full_scan_to_index(self._wallet, scan_to)
            if update is not None:
                self._wallet.apply_update(update)
            tip = client.tip()
            facts = self._facts(self._wallet, tip, prior_state)
        except AppError:
            raise
        except Exception as exc:
            safe = redact_operational_text(redact_secret_text(str(exc)))
            raise AppError(
                f"LWK Liquid observation failed for backend '{self.backend.get('name')}'",
                code="backend_sync_failed",
                hint="Check the backend connection and retry; Kassiber did not apply partial observer state.",
                details={"dependency": "lwk", "error": safe}, retryable=True,
            ) from exc
        return {
            "state": {"schema_version": LWK_OBSERVER_STATE_VERSION, "canonical_txids": list(facts.freshness_checkpoint["canonical_txids"])},
            "facts": {
                "transaction_records": list(facts.transaction_records),
                "retracted_external_ids": list(facts.retracted_external_ids),
                "outputs": list(facts.outputs),
                "coverage": [
                    {"branch_key": p.branch_key, "scanned_to": p.scanned_to, "highest_used": p.highest_used, "details": dict(p.details or {})}
                    for p in facts.coverage
                ],
                "freshness_checkpoint": dict(facts.freshness_checkpoint),
            },
        }

    def apply(self, prepared_update: Mapping[str, Any], prior_state: StoredObserverState | None) -> ObserverApplication:
        facts = prepared_update["facts"]
        return ObserverApplication(
            state=dict(prepared_update["state"]),
            facts=ChainFacts(
                transaction_records=tuple(facts["transaction_records"]),
                retracted_external_ids=tuple(facts["retracted_external_ids"]),
                outputs=tuple(facts["outputs"]),
                coverage=tuple(CoveragePoint(**point) for point in facts["coverage"]),
                freshness_checkpoint=dict(facts["freshness_checkpoint"]),
            ),
        )

    def persist_opaque_state(self, conn) -> None:
        self.store.persist(conn)

    def discard(self) -> None:
        self._wallet = None
        self._store_link = None
        self.store.discard()


__all__ = [
    "LWK_OBSERVER_STATE_VERSION",
    "LwkObserver",
    "lwk_compatibility_reason",
    "lwk_descriptor_for_plan",
    "lwk_network",
]
