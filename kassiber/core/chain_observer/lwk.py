"""LWK-backed, observation-only Liquid chain observer."""

from __future__ import annotations

import os
import re
import json
from decimal import Decimal
from typing import Any, Mapping
from urllib import parse as urlparse

from ...backends import backend_batch_size, backend_timeout, backend_value
from ...egress_ledger import endpoint_from_url, get_egress_ledger
from ...envelope import json_ready
from ...errors import AppError
from ...redaction import redact_operational_text, redact_secret_text
from ...time_utils import UNKNOWN_OCCURRED_AT, timestamp_to_iso
from ...proxy import is_onion_endpoint
from ...util import parse_bool
from ...wallet_descriptors import liquid_asset_code
from ..sync import emit_sync_progress, normalize_backend_kind
from .contract import ChainFacts, ObserverApplication, ObserverPrepareRequest
from .identity import ObserverIdentity
from .lwk_persistence import SqlCipherForeignStore, require_lwk
from .store import CoveragePoint, StoredObserverState


LWK_OBSERVER_STATE_VERSION = 1
_LIQUID_BRANCH_STEP_RE = re.compile(r"/(?P<branch>[01])/\*")


def _truthy_env(name: str) -> bool:
    return str(os.environ.get(name) or "").strip().lower() in {"1", "true", "yes", "on"}


def lwk_compatibility_reason(backend: Mapping[str, Any], sync_state: Any) -> str | None:
    """Preflight configurations the Python binding cannot safely represent."""

    if getattr(sync_state, "chain", None) != "liquid":
        return "non_liquid_chain"
    plan = getattr(sync_state, "descriptor_plan", None)
    if plan is None:
        return "address_list"
    kind = normalize_backend_kind(backend.get("kind"))
    if kind == "esplora" and backend_value(backend, "certificate"):
        # The compatibility HTTP transport cannot load a per-backend trust
        # root either, so reject this before missing LWK wheels can redirect
        # the wallet to an unsafe compatibility route.
        raise AppError(
            "LWK Esplora does not support a configured custom trust root",
            code="observer_capability_unsupported",
            hint="Use platform trust or an Electrum backend until LWK exposes per-client custom CA support.",
            details={"capability": "esplora_custom_ca", "observer": "lwk"},
            retryable=False,
        )
    try:
        require_lwk()
    except AppError as exc:
        if exc.code == "dependency_missing":
            return "dependency_unavailable"
        raise
    if kind not in {"esplora", "electrum"}:
        return "backend_kind"
    endpoint = str(backend.get("url") or "")
    proxy = backend_value(backend, "tor_proxy", "proxy")
    if proxy or is_onion_endpoint(endpoint):
        return "proxy_transport"
    if kind == "electrum" and backend_value(backend, "certificate"):
        return "custom_ca"
    if kind == "electrum" and parse_bool(
        backend_value(backend, "insecure"), default=False
    ):
        # LWK 0.18.0 pins rust-electrum-client 0.21.0. Its Rustls
        # NoCertificateVerification implementation advertises no supported
        # signature schemes, so the explicit validate_domain=False path cannot
        # complete a standards-compliant TLS handshake. Keep the working
        # compatibility transport until the packaged binding contains the
        # upstream verifier fix.
        return "insecure_tls"
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
        selectors = tuple(getattr(branch, "selector", None) for branch in branches)
        if selectors != (0, 1) and _canonical_liquid_multipath_text(plan) is None:
            return "separate_change_descriptor"
    try:
        lwk_descriptor_for_plan(plan)
    except Exception:
        return "descriptor_unsupported"
    return None


def _lwk_esplora_auth_options(lwk: Any, backend: Mapping[str, Any]) -> dict[str, Any]:
    """Translate encrypted Kassiber credentials into LWK builder options."""

    options: dict[str, Any] = {}
    auth_header = str(backend_value(backend, "auth_header") or "").strip()
    token = str(backend_value(backend, "token") or "").strip()
    if auth_header:
        options["headers"] = {"Authorization": auth_header}
    if token:
        options["token_provider"] = lwk.TokenProvider.STATIC(token)
    return options


def _lwk_electrum_connection(backend: Mapping[str, Any]) -> tuple[str, bool, bool]:
    """Return endpoint, TLS use and domain validation for the explicit client."""

    raw = str(backend.get("url") or "").strip()
    parsed = urlparse.urlsplit(raw if "://" in raw else f"ssl://{raw}")
    scheme = str(parsed.scheme or "ssl").lower()
    if scheme not in {"ssl", "tls", "tcp"}:
        raise AppError(
            f"Unsupported LWK Electrum transport '{scheme}'",
            code="observer_capability_unsupported",
            retryable=False,
        )
    host = parsed.hostname
    port = parsed.port or (50002 if scheme in {"ssl", "tls"} else 50001)
    if not host or not port:
        raise AppError("Invalid LWK Electrum endpoint", code="validation", retryable=False)
    tls = scheme in {"ssl", "tls"}
    endpoint = f"[{host}]:{port}" if ":" in host else f"{host}:{port}"
    return endpoint, tls, tls


def _lwk_scan_to_index(
    plan: Any,
    prior_state: StoredObserverState | None,
    checkpoint: Mapping[str, Any],
) -> int:
    """Keep the explicit LWK scan horizon ahead of prior discoveries."""

    gap_limit = max(1, int(plan.gap_limit))
    highest_used: list[int] = []
    if prior_state is not None:
        highest_used.extend(
            int(point.highest_used)
            for point in prior_state.coverage
            if point.highest_used is not None and int(point.highest_used) >= 0
        )
    for value in (checkpoint.get("highest_used") or {}).values():
        try:
            parsed = int(value)
        except (TypeError, ValueError):
            continue
        if parsed >= 0:
            highest_used.append(parsed)
    return max([gap_limit - 1, *(value + gap_limit for value in highest_used)])


def _lwk_coverage(
    *,
    branch_keys: tuple[str, ...],
    scan_to_index: int,
    highest_used: Mapping[str, int],
) -> tuple[CoveragePoint, ...]:
    """Return the exact exclusive boundary submitted to LWK's full scan."""

    requested_index = int(scan_to_index)
    if requested_index < 0:
        raise AppError(
            "LWK scan horizon must be non-negative",
            code="observer_state_invalid",
            retryable=False,
        )
    points = []
    for branch_key in branch_keys:
        used = highest_used.get(branch_key)
        # LWK documents the request as a minimum and may scan farther using
        # wollet state. A discovered index proves only that branch was scanned
        # through that point, not that another gap or the other branch was.
        scanned_to = max(
            requested_index + 1,
            int(used) + 1 if used is not None else 0,
        )
        points.append(
            CoveragePoint(
                branch_key=branch_key,
                scanned_to=scanned_to,
                highest_used=used,
                details={"observer": "lwk"},
            )
        )
    return tuple(points)


def _canonical_liquid_multipath_text(plan: Any) -> str | None:
    """Combine only provably-equivalent separate receive/change descriptors."""

    branches = tuple(getattr(plan, "branches", ()))
    if len(branches) != 2:
        return None
    receive = str(branches[0].descriptor)
    change = str(branches[1].descriptor)
    receive_steps = list(_LIQUID_BRANCH_STEP_RE.finditer(receive))
    change_steps = list(_LIQUID_BRANCH_STEP_RE.finditer(change))
    if not receive_steps or len(receive_steps) != len(change_steps):
        return None
    if any(match.group("branch") != "0" for match in receive_steps):
        return None
    if any(match.group("branch") != "1" for match in change_steps):
        return None
    receive_shape = _LIQUID_BRANCH_STEP_RE.sub("/{branch}/*", receive)
    change_shape = _LIQUID_BRANCH_STEP_RE.sub("/{branch}/*", change)
    if receive_shape != change_shape:
        return None
    return _LIQUID_BRANCH_STEP_RE.sub("/<0;1>/*", receive)


def _lwk_descriptor_text_for_plan(plan: Any) -> str:
    branches = tuple(getattr(plan, "branches", ()))
    if len(branches) == 2 and str(branches[0].descriptor) != str(branches[1].descriptor):
        canonical = _canonical_liquid_multipath_text(plan)
        if canonical is None:
            raise AppError(
                "Separate Liquid receive/change descriptors are not structurally equivalent",
                code="observer_capability_unsupported",
                retryable=False,
            )
        return canonical
    return str(branches[0].descriptor)


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


def _translate_lwk_script_wrapper(text: str) -> str:
    """Translate only the outer Elements script wrapper expected by LWK.

    LWK spells native wrappers as ``elwpkh``/``elwsh``/``eltr``/``elsh``,
    but a nested SegWit redeem script remains Bitcoin miniscript. For example,
    ``sh(wpkh(...))`` must become ``elsh(wpkh(...))``, never
    ``elsh(elwpkh(...))``.
    """

    depth = 0
    script_start = None
    for index, character in enumerate(text):
        if character == "(":
            depth += 1
        elif character == ")":
            depth -= 1
        elif character == "," and depth == 1:
            script_start = index + 1
            break
    if script_start is None:
        raise AppError(
            "LWK confidential descriptor is missing its script expression",
            code="observer_capability_unsupported",
            retryable=False,
        )
    script = text[script_start:]
    for source, target in (
        ("wpkh(", "elwpkh("),
        ("wsh(", "elwsh("),
        ("tr(", "eltr("),
        ("sh(", "elsh("),
    ):
        if script.startswith(source):
            return text[:script_start] + target + script[len(source) :]
    return text


def lwk_descriptor_for_plan(plan: Any):
    """Translate the executable embit CT plan into LWK's CT spelling."""

    lwk = require_lwk()
    descriptor = plan.branches[0].descriptor
    text = _lwk_descriptor_text_for_plan(plan)
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
    text = _translate_lwk_script_wrapper(text)
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


def _allocated_liquid_fee(
    *,
    net_sats: int,
    transaction_fee_sats: int,
    owns_input: bool,
    mixed_inputs: bool,
) -> tuple[int, bool]:
    """Return wallet-attributed fee and whether it remains folded into delta."""

    if not owns_input or net_sats >= 0 or transaction_fee_sats <= 0:
        return 0, False
    if mixed_inputs:
        return 0, True
    return int(transaction_fee_sats), False


def _require_lwk_tip_not_behind(tip: Any, persisted_height: int) -> None:
    remote_height = int(tip.height())
    if remote_height < int(persisted_height):
        raise AppError(
            "The selected Liquid backend is behind the persisted wallet state",
            code="backend_tip_behind",
            hint="Wait for the backend to catch up or select a fully synchronized backend, then retry.",
            details={
                "observer": "lwk",
                "backend_height": remote_height,
                "persisted_height": int(persisted_height),
            },
            retryable=True,
        )


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

    def _identity_checkpoint(self, checkpoint: Mapping[str, Any]) -> Mapping[str, Any]:
        instances = checkpoint.get("observer_instances")
        if isinstance(instances, Mapping):
            instance = instances.get(self.identity.id)
            return instance if isinstance(instance, Mapping) else {}
        return checkpoint

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
        kind = normalize_backend_kind(self.backend["kind"])
        if kind == "esplora":
            builder = lwk.EsploraClientBuilder(
                base_url=endpoint,
                network=network,
                concurrency=max(1, min(8, backend_batch_size(self.backend))),
                timeout=max(1, min(255, backend_timeout(self.backend))),
                **_lwk_esplora_auth_options(lwk, self.backend),
            )
            return lwk.EsploraClient.from_builder(builder)
        if kind == "electrum":
            electrum_endpoint, tls, validate_domain = _lwk_electrum_connection(self.backend)
            return lwk.ElectrumClient(electrum_endpoint, tls, validate_domain)
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
        mixed_inputs = owns_input and sum(item is not None for item in owned_inputs) < len(
            raw_inputs
        )
        records = []
        for asset_id, signed_value in sorted(wallet_tx.balance().items(), key=lambda item: str(item[0])):
            asset_id = str(asset_id)
            net = int(signed_value)
            transaction_fee = fee_sats_by_asset.get(asset_id, 0)
            fee, amount_includes_fee = _allocated_liquid_fee(
                net_sats=net,
                transaction_fee_sats=transaction_fee,
                owns_input=owns_input,
                mixed_inputs=mixed_inputs,
            )
            if net == 0 and fee == 0:
                continue
            if net > 0:
                direction, amount, record_fee, kind = "inbound", _btc(net), Decimal(0), "deposit"
            else:
                direction = "outbound"
                amount = _btc(abs(net) if amount_includes_fee else max(0, abs(net) - fee))
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
                    "amount_includes_fee": amount_includes_fee,
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
                                    "transaction_fee_sats": transaction_fee,
                                    "fee_attribution": (
                                        "implicit_wallet_delta"
                                        if amount_includes_fee
                                        else "exact"
                                    ),
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

    def _facts(
        self,
        wollet: Any,
        tip: Any,
        prior_state: StoredObserverState | None,
        *,
        scan_to_index: int,
    ) -> ChainFacts:
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
        coverage = _lwk_coverage(
            branch_keys=self.identity.branch_keys,
            scan_to_index=scan_to_index,
            highest_used=highest,
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
            identity_checkpoint = self._identity_checkpoint(request.checkpoint)
            if prior_state is not None and prior_state.payload.get("schema_version") != LWK_OBSERVER_STATE_VERSION:
                if not request.force_full:
                    raise AppError("Stored LWK observer state must be rebuilt", code="observer_state_rebuild_required", retryable=False)
                prior_state = None
            retraction_state = prior_state
            if retraction_state is None and request.force_full:
                retraction_state = StoredObserverState(
                    identity=self.identity,
                    payload={
                        "canonical_txids": list(
                            identity_checkpoint.get("canonical_txids") or ()
                        )
                    },
                    coverage=(),
                )
            lwk = require_lwk()
            network = lwk_network(self.identity.network, self.policy_asset_id)
            self._store_link = lwk.ForeignStoreLink(self.store)
            self._wallet = lwk.Wollet.with_custom_store(network, lwk_descriptor_for_plan(self.plan), self._store_link)
            client = self._client(network)
            emit_sync_progress({"phase": "backend_fetch", "observer": "lwk", "status": "scanning"})
            if request.force_full and identity_checkpoint.get("tip_height") is not None:
                # Rebuilding the opaque wollet must not let a lagging backend
                # retract facts newer than its current chain view.
                _require_lwk_tip_not_behind(
                    client.tip(),
                    int(identity_checkpoint["tip_height"]),
                )
            scan_to = _lwk_scan_to_index(self.plan, prior_state, identity_checkpoint)
            update = client.full_scan_to_index(self._wallet, scan_to)
            if update is not None:
                self._wallet.apply_update(update)
            tip = client.tip()
            facts = self._facts(
                self._wallet,
                tip,
                retraction_state,
                scan_to_index=scan_to,
            )
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
