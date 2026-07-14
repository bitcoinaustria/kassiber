"""BDK-backed, observation-only Bitcoin chain observer."""

from __future__ import annotations

import hashlib
import os
import struct
from dataclasses import dataclass
from typing import Any, Mapping, Sequence

from ...backends import backend_batch_size, backend_timeout, backend_value
from ...egress_ledger import endpoint_from_url, get_egress_ledger
from ...errors import AppError
from ...proxy import is_onion_endpoint
from ...redaction import redact_operational_text, redact_secret_text
from ...wallet_descriptors import branch_descriptor
from ..sync import emit_sync_progress
from .bdk_persistence import SqlCipherBdkPersistence, deserialize_changeset
from .contract import ChainFacts, ObserverApplication, ObserverPrepareRequest
from .identity import ObserverIdentity
from .store import CoveragePoint, StoredObserverState


BDK_OBSERVER_STATE_VERSION = 1


def _truthy_env(name: str) -> bool:
    return str(os.environ.get(name) or "").strip().lower() in {"1", "true", "yes", "on"}


def _network(value: str) -> Any:
    import bdkpython as bdk

    return {
        "main": bdk.Network.BITCOIN,
        "test": bdk.Network.TESTNET,
        "testnet": bdk.Network.TESTNET,
        "testnet4": bdk.Network.TESTNET4,
        "signet": bdk.Network.SIGNET,
        "regtest": bdk.Network.REGTEST,
    }[str(value).strip().lower()]


def _network_kind(network: Any) -> Any:
    import bdkpython as bdk

    return bdk.NetworkKind.MAIN if network == bdk.Network.BITCOIN else bdk.NetworkKind.TEST


def _electrum_header_hash(header: Any) -> str:
    """Return the display-order block hash for a BDK Electrum header."""

    payload = b"".join(
        (
            struct.pack("<I", int(header.version) & 0xFFFFFFFF),
            bytes.fromhex(str(header.prev_blockhash))[::-1],
            bytes.fromhex(str(header.merkle_root))[::-1],
            struct.pack("<III", int(header.time), int(header.bits), int(header.nonce)),
        )
    )
    return hashlib.sha256(hashlib.sha256(payload).digest()).digest()[::-1].hex()


def _electrum_confirmation_rebuild_needed(wallet: Any, prior_tip_height: int | None) -> bool:
    """Work around stale mempool positions in bdk_electrum 0.23.2.

    The binding bundled by bdkpython 3.0.0 can advance the local chain without
    replacing an already-known mempool position with its new anchor.  A fresh
    dependency scan is necessary only when the tip advanced and an
    unconfirmed canonical transaction remains.
    """

    if prior_tip_height is None or int(wallet.latest_checkpoint().height) <= prior_tip_height:
        return False
    return any(not item.chain_position.is_confirmed() for item in wallet.transactions())


def bdk_compatibility_reason(backend: Mapping[str, Any], sync_state: Any) -> str | None:
    """Return why an existing adapter must remain in use, or ``None`` for BDK."""

    if getattr(sync_state, "chain", None) != "bitcoin":
        return "non_bitcoin_chain"
    plan = getattr(sync_state, "descriptor_plan", None)
    if plan is None:
        return "address_list"
    if getattr(plan, "kind", None) == "silent-payment":
        return "silent_payment"
    kind = str(backend.get("kind") or "").strip().lower()
    if kind not in {"esplora", "electrum"}:
        return "backend_kind"
    if backend_value(backend, "certificate"):
        # bdk_esplora and bdk_electrum do not expose a custom trust-store hook.
        return "custom_ca"
    if kind == "esplora" and (
        backend_value(backend, "auth_header") or backend_value(backend, "token")
    ):
        return "custom_http_auth"
    if kind == "esplora" and backend_value(backend, "timeout") is not None:
        try:
            if int(backend_value(backend, "timeout")) != 30:
                return "custom_timeout"
        except (TypeError, ValueError):
            return "custom_timeout"
    return None


def bdk_plan_compatibility_reason(backend: Mapping[str, Any], plan: Any) -> str | None:
    class _State:
        chain = getattr(plan, "chain", None)
        descriptor_plan = plan

    return bdk_compatibility_reason(backend, _State())


@dataclass(frozen=True, slots=True)
class BdkBranch:
    branch_key: str
    branch_index: int
    branch_label: str
    descriptor: str
    internal: bool


def bdk_branches_for_identity(plan: Any, identity: ObserverIdentity) -> tuple[BdkBranch, ...]:
    branches = list(plan.branches)
    if identity.source_key.startswith("xpub:"):
        script_type = identity.source_key.split(":", 1)[1]
        branches = [
            branch
            for branch in branches
            if str(branch.branch_label).lower().startswith(f"{script_type} ")
        ]
    output: list[BdkBranch] = []
    for position, branch in enumerate(branches):
        label = str(branch.branch_label)
        internal = "change" in label.lower() or position == 1
        output.append(
            BdkBranch(
                branch_key="change" if internal else "receive",
                branch_index=int(branch.branch_index),
                branch_label=label,
                descriptor=str(branch_descriptor(branch)),
                internal=internal,
            )
        )
    if not output or len(output) > 2 or sum(not item.internal for item in output) != 1:
        raise AppError(
            "The Bitcoin descriptor source cannot be represented by one BDK observer",
            code="observer_capability_unsupported",
            details={"source_key": identity.source_key, "branch_count": len(output)},
            retryable=False,
        )
    if sum(item.internal for item in output) > 1:
        raise AppError(
            "The Bitcoin descriptor source has more than one change branch",
            code="observer_capability_unsupported",
            retryable=False,
        )
    return tuple(output)


class BdkObserver:
    def __init__(
        self,
        *,
        identity: ObserverIdentity,
        backend: Mapping[str, Any],
        branches: Sequence[BdkBranch],
        gap_limit: int,
    ):
        self.identity = identity
        self.backend = dict(backend)
        self.branches = tuple(branches)
        self.gap_limit = max(1, int(gap_limit))
        self._wallet = None
        self._persistence = None

    def _client(self) -> Any:
        import bdkpython as bdk

        endpoint = str(self.backend.get("url") or "").strip()
        proxy = str(backend_value(self.backend, "tor_proxy", "proxy") or "").strip() or None
        if not endpoint:
            raise AppError("BDK observer backend is missing its endpoint", code="validation")
        if is_onion_endpoint(endpoint) and proxy is None:
            raise AppError(
                ".onion backend URLs require a Tor/SOCKS proxy",
                code="network_proxy_required",
                hint="Configure a SOCKS proxy; Kassiber will not connect to onion services directly.",
                retryable=False,
            )
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
            operation=f"bdk.{self.backend['kind']}.connect",
            via_proxy=proxy is not None,
        )
        kind = str(self.backend["kind"]).lower()
        if kind == "esplora":
            return bdk.EsploraClient(endpoint, proxy=proxy)
        if kind == "electrum":
            return bdk.ElectrumClient(
                endpoint,
                socks5=proxy,
                timeout=max(1, int(backend_timeout(self.backend))),
                retry=1,
                validate_domain=not bool(backend_value(self.backend, "insecure")),
            )
        raise AppError("Unsupported BDK observer backend", code="observer_capability_unsupported")

    def _wallet_from_state(self, prior_state: StoredObserverState | None) -> tuple[Any, Any]:
        import bdkpython as bdk

        aggregate = None
        if prior_state is not None:
            if prior_state.payload.get("schema_version") != BDK_OBSERVER_STATE_VERSION:
                raise AppError(
                    "Stored BDK observer state must be rebuilt",
                    code="observer_state_rebuild_required",
                    hint="Run a full wallet refresh to rebuild derived BDK observer state.",
                    retryable=False,
                )
            aggregate_payload = prior_state.payload.get("bdk_changeset")
            if not isinstance(aggregate_payload, Mapping):
                raise AppError(
                    "Stored BDK observer state is incomplete",
                    code="observer_state_rebuild_required",
                    retryable=False,
                )
            aggregate = deserialize_changeset(aggregate_payload)
        persistence = SqlCipherBdkPersistence(aggregate)
        self._persistence = persistence
        persister = bdk.Persister.custom(persistence)
        network = _network(self.identity.network)
        external = next(item for item in self.branches if not item.internal)
        internal = next((item for item in self.branches if item.internal), None)
        descriptor = bdk.Descriptor(external.descriptor, _network_kind(network))
        change_descriptor = (
            bdk.Descriptor(internal.descriptor, _network_kind(network))
            if internal is not None
            else None
        )
        if prior_state is None:
            wallet = (
                bdk.Wallet(descriptor, change_descriptor, network, persister, self.gap_limit)
                if change_descriptor is not None
                else bdk.Wallet.create_single(descriptor, network, persister, self.gap_limit)
            )
        else:
            wallet = (
                bdk.Wallet.load(descriptor, change_descriptor, persister, self.gap_limit)
                if change_descriptor is not None
                else bdk.Wallet.load_single(descriptor, persister, self.gap_limit)
            )
        return wallet, persister

    def _remote_block_hash(self, client: Any, height: int) -> str | None:
        kind = str(self.backend["kind"]).lower()
        if kind == "esplora":
            remote_height = int(client.get_height())
            return str(client.get_block_hash(height)) if remote_height >= height else None
        notification = client.block_headers_subscribe()
        if int(notification.height) < height:
            return None
        return _electrum_header_hash(client.block_header(height))

    def _target(self, wallet: Any, script: Any) -> dict[str, Any] | None:
        import bdkpython as bdk

        derivation = wallet.derivation_of_spk(script)
        if derivation is None:
            return None
        internal = derivation.keychain == bdk.KeychainKind.INTERNAL
        branch = next((item for item in self.branches if item.internal == internal), None)
        if branch is None:
            return None
        try:
            address = str(bdk.Address.from_script(script, _network(self.identity.network)))
        except Exception:
            address = None
        return {
            "chain": "bitcoin",
            "network": self.identity.network,
            "branch_index": branch.branch_index,
            "branch_label": branch.branch_label,
            "address_index": int(derivation.index),
            "address": address,
            "script_pubkey": bytes(script.to_bytes()).hex(),
            "derivation_path": None,
            "derivation_paths": [],
            "key_origins": [],
        }

    @staticmethod
    def _position(position: Any, tip_height: int) -> dict[str, Any]:
        if position.is_confirmed():
            block_time = position.confirmation_block_time
            height = int(block_time.block_id.height)
            return {
                "status": {
                    "confirmed": True,
                    "block_height": height,
                    "block_hash": str(block_time.block_id.hash),
                    "block_time": int(block_time.confirmation_time),
                },
                "confirmation_status": "confirmed",
                "block_height": height,
                "block_time": int(block_time.confirmation_time),
                "confirmations": max(0, tip_height - height + 1),
            }
        return {
            "status": {"confirmed": False},
            "confirmation_status": "mempool",
            "block_height": None,
            "block_time": None,
            "confirmations": 0,
        }

    def _facts(self, wallet: Any, prior_state: StoredObserverState | None) -> ChainFacts:
        from ..sync_backends import record_from_bitcoin_esplora_tx

        graph = self._persistence.aggregate.tx_graph_changeset()
        transactions = {str(tx.compute_txid()): tx for tx in graph.txs}
        first_seen = {str(txid): int(timestamp) for txid, timestamp in graph.first_seen.items()}
        txouts: dict[tuple[str, int], Any] = {}
        for txid, tx in transactions.items():
            for vout, output in enumerate(tx.output()):
                txouts[(txid, vout)] = output
        for key, output in graph.txouts.items():
            outpoint = key.outpoint()
            txouts[(str(outpoint.txid), int(outpoint.vout))] = output

        tracked: dict[str, Mapping[str, Any]] = {}
        for output in txouts.values():
            target = self._target(wallet, output.script_pubkey)
            if target is not None:
                tracked[target["script_pubkey"]] = target

        tip_height = int(wallet.latest_checkpoint().height)
        records = []
        canonical_txids = []
        for canonical in wallet.transactions():
            tx = canonical.transaction
            txid = str(tx.compute_txid())
            canonical_txids.append(txid)
            details = wallet.tx_details(tx.compute_txid())
            position = self._position(canonical.chain_position, tip_height)
            vin = []
            for txin in tx.input():
                outpoint = txin.previous_output
                prevout = txouts.get((str(outpoint.txid), int(outpoint.vout)))
                vin.append(
                    {
                        "txid": str(outpoint.txid),
                        "vout": int(outpoint.vout),
                        "prevout": (
                            {
                                "value": int(prevout.value.to_sat()),
                                "scriptpubkey": bytes(prevout.script_pubkey.to_bytes()).hex(),
                            }
                            if prevout is not None
                            else None
                        ),
                        "witness": [bytes(item).hex() for item in txin.witness],
                    }
                )
            raw = {
                "txid": txid,
                "vin": vin,
                "vout": [
                    {
                        "value": int(output.value.to_sat()),
                        "scriptpubkey": bytes(output.script_pubkey.to_bytes()).hex(),
                    }
                    for output in tx.output()
                ],
                "fee": int(details.fee.to_sat()) if details is not None and details.fee is not None else 0,
                "observed_at": first_seen.get(txid),
                **position,
                "observer": "bdk",
                "observer_owned_scripts": sorted(tracked),
            }
            record = record_from_bitcoin_esplora_tx(raw, tracked, str(self.backend["name"]))
            if record is not None:
                # Observer updates cross a JSON-only boundary. Decimal text is
                # exact and the importer already accepts it without a float
                # round-trip.
                record["amount"] = str(record["amount"])
                record["fee"] = str(record["fee"])
                records.append(record)

        outputs = []
        highest_used: dict[bool, int] = {}
        for local in wallet.list_output():
            highest_used[local.keychain.name == "INTERNAL"] = max(
                highest_used.get(local.keychain.name == "INTERNAL", -1),
                int(local.derivation_index),
            )
        for local in wallet.list_unspent():
            target = self._target(wallet, local.txout.script_pubkey)
            if target is None:
                continue
            position = self._position(local.chain_position, tip_height)
            outputs.append(
                {
                    "txid": str(local.outpoint.txid),
                    "vout": int(local.outpoint.vout),
                    "asset": "BTC",
                    "amount_sats": int(local.txout.value.to_sat()),
                    "confirmation_status": position["confirmation_status"],
                    "confirmations": position["confirmations"],
                    "block_height": position["block_height"],
                    "block_time": position["block_time"],
                    "chain": "bitcoin",
                    "network": self.identity.network,
                    **target,
                    "raw": {"source": "bdk_wallet", "confirmed": position["status"]["confirmed"]},
                }
            )

        coverage = []
        checkpoint_highest = {}
        import bdkpython as bdk

        for branch in self.branches:
            keychain = bdk.KeychainKind.INTERNAL if branch.internal else bdk.KeychainKind.EXTERNAL
            used = highest_used.get(branch.internal)
            revealed = wallet.derivation_index(keychain)
            scanned_to = max(
                self.gap_limit,
                (int(revealed) + self.gap_limit + 1) if revealed is not None else self.gap_limit,
                (int(used) + self.gap_limit + 1) if used is not None else self.gap_limit,
            )
            coverage.append(
                CoveragePoint(
                    branch_key=branch.branch_key,
                    scanned_to=scanned_to,
                    highest_used=used,
                    details={"branch_index": branch.branch_index, "observer": "bdk"},
                )
            )
            if used is not None:
                checkpoint_highest[str(branch.branch_index)] = int(used)
        previous_txids = set(
            str(value)
            for value in ((prior_state.payload.get("canonical_txids") or []) if prior_state else [])
        )
        current_txids = set(canonical_txids)
        return ChainFacts(
            transaction_records=tuple(records),
            retracted_external_ids=tuple(sorted(previous_txids - current_txids)),
            outputs=tuple(outputs),
            coverage=tuple(coverage),
            freshness_checkpoint={
                "observer": "bdk",
                "tip_height": tip_height,
                "tip_hash": str(wallet.latest_checkpoint().hash),
                "highest_used": dict(sorted(checkpoint_highest.items())),
                "canonical_txids": sorted(current_txids),
            },
        )

    def prepare(
        self,
        request: ObserverPrepareRequest,
        prior_state: StoredObserverState | None,
    ) -> Mapping[str, Any]:
        try:
            retraction_state = prior_state
            if retraction_state is None and request.force_full:
                retraction_state = StoredObserverState(
                    identity=self.identity,
                    payload={
                        "canonical_txids": list(
                            request.checkpoint.get("canonical_txids") or ()
                        )
                    },
                    coverage=(),
                )
            wallet_state = None if request.force_full else prior_state
            wallet, persister = self._wallet_from_state(wallet_state)
            self._wallet = wallet
            prior_tip_height = (
                int(wallet.latest_checkpoint().height) if wallet_state is not None else None
            )
            emit_sync_progress({"phase": "backend_fetch", "observer": "bdk", "status": "scanning"})
            client = self._client()
            if wallet_state is not None:
                prior_tip = wallet.latest_checkpoint()
                remote_hash = self._remote_block_hash(client, int(prior_tip.height))
                if remote_hash != str(prior_tip.hash):
                    # bdk_electrum bundled by bdkpython 3.0.0 predates the
                    # upstream stale-anchor reorg fix. Rebuilding through the
                    # same BDK dependency keeps BDK authoritative while safely
                    # replacing a disconnected local chain/anchor aggregate.
                    wallet, persister = self._wallet_from_state(None)
                    self._wallet = wallet
            scan_request = wallet.start_full_scan().build()
            kind = str(self.backend["kind"]).lower()
            if kind == "esplora":
                update = client.full_scan(
                    scan_request,
                    stop_gap=self.gap_limit,
                    parallel_requests=max(1, min(8, backend_batch_size(self.backend))),
                )
            else:
                update = client.full_scan(
                    scan_request,
                    stop_gap=self.gap_limit,
                    batch_size=max(1, backend_batch_size(self.backend)),
                    fetch_prev_txouts=True,
                )
            wallet.apply_update(update)
            wallet.persist(persister)
            if kind == "electrum" and _electrum_confirmation_rebuild_needed(
                wallet,
                prior_tip_height,
            ):
                # bdk_electrum 0.23.2 may leave a persisted mempool position
                # stale when the transaction becomes confirmed. Build a fresh
                # dependency request for that transition, but apply its update
                # onto the existing aggregate so anchors omitted from the
                # fresh response cannot demote older confirmed transactions.
                aggregate_persistence = self._persistence
                fresh_wallet, _fresh_persister = self._wallet_from_state(None)
                update = client.full_scan(
                    fresh_wallet.start_full_scan().build(),
                    stop_gap=self.gap_limit,
                    batch_size=max(1, backend_batch_size(self.backend)),
                    fetch_prev_txouts=True,
                )
                self._persistence = aggregate_persistence
                wallet.apply_update(update)
                wallet.persist(persister)
            facts = self._facts(wallet, retraction_state)
        except AppError:
            raise
        except Exception as exc:
            safe_error = redact_operational_text(redact_secret_text(str(exc)))
            raise AppError(
                f"BDK Bitcoin observation failed for backend '{self.backend.get('name')}'",
                code="backend_sync_failed",
                hint="Check the backend connection and retry; Kassiber did not apply partial observer state.",
                details={"dependency": "bdkpython", "error": safe_error},
                retryable=True,
            ) from exc
        emit_sync_progress(
            {
                "phase": "decode_enrich",
                "observer": "bdk",
                "transactions_seen": len(facts.transaction_records),
                "utxos_seen": len(facts.outputs),
            }
        )
        return {
            "state": {
                "schema_version": BDK_OBSERVER_STATE_VERSION,
                "bdk_changeset": self._persistence.payload(),
                "canonical_txids": list(facts.freshness_checkpoint.get("canonical_txids") or []),
            },
            "facts": {
                "transaction_records": list(facts.transaction_records),
                "retracted_external_ids": list(facts.retracted_external_ids),
                "outputs": list(facts.outputs),
                "coverage": [
                    {
                        "branch_key": point.branch_key,
                        "scanned_to": point.scanned_to,
                        "highest_used": point.highest_used,
                        "details": dict(point.details or {}),
                    }
                    for point in facts.coverage
                ],
                "freshness_checkpoint": dict(facts.freshness_checkpoint),
            },
        }

    def apply(
        self,
        prepared_update: Mapping[str, Any],
        prior_state: StoredObserverState | None,
    ) -> ObserverApplication:
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

    def discard(self) -> None:
        self._wallet = None
        self._persistence = None


__all__ = [
    "BDK_OBSERVER_STATE_VERSION",
    "BdkBranch",
    "BdkObserver",
    "bdk_branches_for_identity",
    "bdk_compatibility_reason",
    "bdk_plan_compatibility_reason",
]
