"""Explicit BDK ``ChangeSet`` persistence inside Kassiber's SQLCipher store.

BDK receives a custom persistence callback backed by an in-memory aggregate.
The aggregate crosses Kassiber's observer boundary as stable, versioned JSON;
BDK is never given a database path and therefore cannot create a second
plaintext SQLite database beside the encrypted book.
"""

from __future__ import annotations

from importlib.metadata import PackageNotFoundError, version
from typing import Any, Mapping

from ...errors import AppError


BDK_CHANGESET_SCHEMA_VERSION = 1
_TOP_LEVEL_KEYS = frozenset(
    {
        "schema_version",
        "descriptor",
        "change_descriptor",
        "network",
        "local_chain",
        "tx_graph",
        "indexer",
        "locked_outpoints",
    }
)
_TX_GRAPH_KEYS = frozenset(
    {"txs", "txouts", "anchors", "last_seen", "first_seen", "last_evicted"}
)


def _bdk():
    try:
        import bdkpython
    except ModuleNotFoundError as exc:
        raise AppError(
            "Bitcoin descriptor refresh requires the bundled BDK dependency",
            code="dependency_missing",
            hint="Install Kassiber from a build that includes bdkpython 3.0.0.",
            details={"missing_package": "bdkpython"},
            retryable=False,
        ) from exc
    try:
        installed_version = version("bdkpython")
    except PackageNotFoundError:
        installed_version = None
    if installed_version != "3.0.0":
        raise AppError(
            "The installed BDK binding does not match Kassiber's observer format",
            code="dependency_version_mismatch",
            details={
                "package": "bdkpython",
                "expected": "3.0.0",
                "actual": installed_version,
            },
            retryable=False,
        )
    return bdkpython


def _hex(value: Any) -> str:
    return bytes(value.serialize()).hex()


def _outpoint_payload(value: Any) -> dict[str, Any]:
    return {"txid": _hex(value.txid), "vout": int(value.vout)}


def serialize_changeset(changeset: Any) -> dict[str, Any]:
    """Serialize every aggregate component without pickle or opaque blobs."""

    local_chain = changeset.localchain_changeset()
    tx_graph = changeset.tx_graph_changeset()
    indexer = changeset.indexer_changeset()
    locked = changeset.locked_outpoints_changeset()

    txouts = []
    for key, txout in tx_graph.txouts.items():
        outpoint = key.outpoint()
        txouts.append(
            {
                **_outpoint_payload(outpoint),
                "value_sats": int(txout.value.to_sat()),
                "script_pubkey": bytes(txout.script_pubkey.to_bytes()).hex(),
            }
        )
    anchors = []
    for anchor in tx_graph.anchors:
        block_time = anchor.confirmation_block_time
        anchors.append(
            {
                "txid": _hex(anchor.txid),
                "block_height": int(block_time.block_id.height),
                "block_hash": _hex(block_time.block_id.hash),
                "confirmation_time": int(block_time.confirmation_time),
            }
        )

    def times(values: Mapping[Any, int]) -> list[dict[str, Any]]:
        return sorted(
            ({"txid": _hex(txid), "timestamp": int(timestamp)} for txid, timestamp in values.items()),
            key=lambda item: item["txid"],
        )

    descriptor = changeset.descriptor()
    change_descriptor = changeset.change_descriptor()
    network = changeset.network()
    return {
        "schema_version": BDK_CHANGESET_SCHEMA_VERSION,
        "descriptor": str(descriptor) if descriptor is not None else None,
        "change_descriptor": str(change_descriptor) if change_descriptor is not None else None,
        "network": network.name if network is not None else None,
        "local_chain": sorted(
            (
                {
                    "height": int(change.height),
                    "hash": _hex(change.hash) if change.hash is not None else None,
                }
                for change in local_chain.changes
            ),
            key=lambda item: item["height"],
        ),
        "tx_graph": {
            "txs": sorted(bytes(tx.serialize()).hex() for tx in tx_graph.txs),
            "txouts": sorted(txouts, key=lambda item: (item["txid"], item["vout"])),
            "anchors": sorted(
                anchors,
                key=lambda item: (item["txid"], item["block_height"], item["block_hash"]),
            ),
            "last_seen": times(tx_graph.last_seen),
            "first_seen": times(tx_graph.first_seen),
            "last_evicted": times(tx_graph.last_evicted),
        },
        "indexer": sorted(
            (
                {"descriptor_id": _hex(descriptor_id), "last_revealed": int(last_revealed)}
                for descriptor_id, last_revealed in indexer.last_revealed.items()
            ),
            key=lambda item: item["descriptor_id"],
        ),
        "locked_outpoints": sorted(
            (
                {**_outpoint_payload(key.outpoint()), "locked": bool(is_locked)}
                for key, is_locked in locked.items()
            ),
            key=lambda item: (item["txid"], item["vout"]),
        ),
    }


def _invalid(message: str, *, field: str | None = None) -> AppError:
    return AppError(
        message,
        code="observer_state_rebuild_required",
        hint="Run a full wallet refresh to rebuild derived BDK observer state.",
        details={"representation": "bdk_changeset", **({"field": field} if field else {})},
        retryable=False,
    )


def _object(value: Any, *, field: str, keys: frozenset[str] | None = None) -> dict[str, Any]:
    if not isinstance(value, Mapping):
        raise _invalid("Stored BDK observer state has an invalid object", field=field)
    output = dict(value)
    if keys is not None and frozenset(output) != keys:
        raise _invalid("Stored BDK observer state has unsupported fields", field=field)
    return output


def _items(value: Any, *, field: str, keys: frozenset[str]) -> list[dict[str, Any]]:
    if not isinstance(value, list):
        raise _invalid("Stored BDK observer state has an invalid list", field=field)
    return [
        _object(item, field=f"{field}[{index}]", keys=keys)
        for index, item in enumerate(value)
    ]


def _boolean(value: Any, *, field: str) -> bool:
    if not isinstance(value, bool):
        raise _invalid("Stored BDK observer state has an invalid boolean", field=field)
    return value


def deserialize_changeset(payload: Mapping[str, Any]) -> Any:
    """Rebuild a BDK aggregate, rejecting missing or unknown components."""

    bdk = _bdk()
    root = _object(payload, field="$", keys=_TOP_LEVEL_KEYS)
    if root.get("schema_version") != BDK_CHANGESET_SCHEMA_VERSION:
        raise _invalid("Stored BDK observer state uses an unsupported schema", field="schema_version")
    try:
        network_name = root.get("network")
        network = bdk.Network[network_name] if network_name is not None else None
        network_kind = (
            bdk.NetworkKind.MAIN if network == bdk.Network.BITCOIN else bdk.NetworkKind.TEST
        )
        descriptor = (
            bdk.Descriptor(str(root["descriptor"]), network_kind)
            if root.get("descriptor") is not None
            else None
        )
        change_descriptor = (
            bdk.Descriptor(str(root["change_descriptor"]), network_kind)
            if root.get("change_descriptor") is not None
            else None
        )
        local_chain = bdk.LocalChainChangeSet(
            changes=[
                bdk.ChainChange(
                    height=int(item["height"]),
                    hash=(
                        bdk.BlockHash.from_bytes(bytes.fromhex(item["hash"]))
                        if item.get("hash") is not None
                        else None
                    ),
                )
                for item in _items(
                    root["local_chain"], field="local_chain", keys=frozenset({"height", "hash"})
                )
            ]
        )
        graph_payload = _object(root["tx_graph"], field="tx_graph", keys=_TX_GRAPH_KEYS)
        txout_items = _items(
            graph_payload["txouts"],
            field="tx_graph.txouts",
            keys=frozenset({"txid", "vout", "value_sats", "script_pubkey"}),
        )
        anchor_items = _items(
            graph_payload["anchors"],
            field="tx_graph.anchors",
            keys=frozenset(
                {"txid", "block_height", "block_hash", "confirmation_time"}
            ),
        )

        def txid(value: str) -> Any:
            return bdk.Txid.from_bytes(bytes.fromhex(value))

        def outpoint(item: Mapping[str, Any]) -> Any:
            return bdk.OutPoint(txid=txid(str(item["txid"])), vout=int(item["vout"]))

        def times(name: str) -> dict[Any, int]:
            return {
                txid(str(item["txid"])): int(item["timestamp"])
                for item in _items(
                    graph_payload[name],
                    field=f"tx_graph.{name}",
                    keys=frozenset({"txid", "timestamp"}),
                )
            }

        tx_graph = bdk.TxGraphChangeSet(
            txs=[bdk.Transaction(bytes.fromhex(value)) for value in graph_payload["txs"]],
            txouts={
                bdk.HashableOutPoint(outpoint(item)): bdk.TxOut(
                    value=bdk.Amount.from_sat(int(item["value_sats"])),
                    script_pubkey=bdk.Script(bytes.fromhex(item["script_pubkey"])),
                )
                for item in txout_items
            },
            anchors=[
                bdk.Anchor(
                    confirmation_block_time=bdk.ConfirmationBlockTime(
                        block_id=bdk.BlockId(
                            height=int(item["block_height"]),
                            hash=bdk.BlockHash.from_bytes(bytes.fromhex(item["block_hash"])),
                        ),
                        confirmation_time=int(item["confirmation_time"]),
                    ),
                    txid=txid(str(item["txid"])),
                )
                for item in anchor_items
            ],
            last_seen=times("last_seen"),
            first_seen=times("first_seen"),
            last_evicted=times("last_evicted"),
        )
        indexer = bdk.IndexerChangeSet(
            last_revealed={
                bdk.DescriptorId.from_bytes(bytes.fromhex(item["descriptor_id"])): int(
                    item["last_revealed"]
                )
                for item in _items(
                    root["indexer"],
                    field="indexer",
                    keys=frozenset({"descriptor_id", "last_revealed"}),
                )
            }
        )
        locked_items = _items(
            root["locked_outpoints"],
            field="locked_outpoints",
            keys=frozenset({"txid", "vout", "locked"}),
        )
        locked = {
            bdk.HashableOutPoint(outpoint(item)): _boolean(
                item["locked"], field=f"locked_outpoints[{index}].locked"
            )
            for index, item in enumerate(locked_items)
        }
    except AppError:
        raise
    except (KeyError, TypeError, ValueError, AttributeError) as exc:
        raise _invalid("Stored BDK observer state cannot be decoded") from exc
    return bdk.ChangeSet.from_aggregate_with_locked_outpoints(
        descriptor,
        change_descriptor,
        network,
        local_chain,
        tx_graph,
        indexer,
        locked,
    )


def SqlCipherBdkPersistence(aggregate: Any | None = None) -> Any:
    """Build BDK's custom callback only after the native route is selected.

    The dependency is intentionally absent on runtimes without a supported
    wheel.  Evaluating ``bdk.Persistence`` as a module-level base class would
    make importing the observer fail before routing can choose compatibility.
    """

    bdk = _bdk()

    class _SqlCipherBdkPersistence(bdk.Persistence):
        def __init__(self, initial: Any | None = None):
            super().__init__()
            self.aggregate = initial if initial is not None else bdk.ChangeSet()

        def initialize(self) -> Any:
            return self.aggregate

        def persist(self, changeset: Any) -> None:
            self.aggregate = bdk.ChangeSet.from_merge(self.aggregate, changeset)

        def payload(self) -> dict[str, Any]:
            return serialize_changeset(self.aggregate)

    return _SqlCipherBdkPersistence(aggregate)


__all__ = [
    "BDK_CHANGESET_SCHEMA_VERSION",
    "SqlCipherBdkPersistence",
    "deserialize_changeset",
    "serialize_changeset",
]
