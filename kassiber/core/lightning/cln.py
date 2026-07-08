"""Core Lightning read-only adapter.

Implements :class:`kassiber.core.lightning.LightningAdapter` against a Core
Lightning node, exposing only the subset of RPC the scaffold's
:class:`NodeSnapshot` needs (``getinfo``, ``listpeers`` / ``listpeerchannels``,
``listforwards``, ``listinvoices``, ``listsendpays`` / ``listpays``,
``listfunds`` for the on-chain balance, and ``bkpr-listincome`` for the income
classification used by the persistence layer below).

Opsec — REQUIRED reading: [docs/reference/lightning-opsec.md](../../../docs/reference/lightning-opsec.md).
The adapter is the discard boundary for Tier-1 sensitive fields:

- ``payment_preimage``, ``payment_secret``, encoded ``bolt11`` blobs, onion
  ``route`` hops, route hints from received invoices, and
  ``failure_source_pubkey`` / ``erring_node`` are dropped at the RPC transport
  boundary via per-resource ``_sanitize_*`` helpers, so the in-memory
  :class:`CoreLightningSnapshot` never carries them. The :class:`NodeSnapshot`
  shapes do not even have somewhere to put them.
- Private channels (``private=true``) surface with ``peer_pubkey=None`` —
  the peer chose private gossip for a reason and Kassiber will not undo that
  decision unless the operator explicitly opts in.
- ``listforwards`` is aggregated at the day-per-channel grain when persisted
  so the DB never holds a complete log of "X paid Y through me" patterns.

Persistence (the optional ``sync_core_lightning_wallet`` entry point) stores
only the curated shapes returned by this module — there is no ``raw_json``
column for full RPC dumps.
"""

from __future__ import annotations

import hashlib
import json
import os
import sqlite3
import subprocess
import uuid
from dataclasses import dataclass
from typing import Any, Callable, Mapping, MutableMapping, Sequence

from ...backends import backend_timeout, backend_value, redact_backend_url
from ...envelope import json_ready
from ...errors import AppError
from ...msat import msat_to_btc
from ...time_utils import UNKNOWN_OCCURRED_AT, now_iso, timestamp_to_iso
from ...util import str_or_none
from .. import imports as core_imports
from ..repo import invalidate_journals
from .capabilities import LightningCapabilities
from .registry import register_adapter
from .types import (
    NodeChannel,
    NodeChannelState,
    NodeForward,
    NodeForwardStatus,
    NodeRoutingSnapshot,
    NodeSnapshot,
)

CLN_BACKEND_KIND = "coreln"
CLN_WALLET_KIND = "coreln"
CLN_IMPORT_SOURCE = "core-lightning"

#: RPC methods the adapter is willing to call. Every method here is read-only
#: by construction; no payment, channel-mutation, signing, or wallet-mutation
#: methods appear. ``call_core_lightning`` enforces this allowlist at the
#: transport boundary so an over-broad rune still cannot ask the adapter to
#: send money or close a channel.
CLN_ALLOWED_METHODS: tuple[str, ...] = (
    "getinfo",
    "listfunds",
    "listpeerchannels",
    "listforwards",
    "listpays",
    "listinvoices",
    "bkpr-listincome",
    "bkpr-listbalances",
    "bkpr-listaccountevents",
)

#: Suggested restriction list for a least-privilege commando rune. Pair it
#: with a ``rate=60`` cap when generating the rune so a compromised reader
#: cannot enumerate the bookkeeper at arbitrary speed.
CLN_READONLY_RUNE_RESTRICTIONS = (
    '[["method^list","method^get","method^bkpr-list","method=summary"],'
    '["method/listdatastore"],["rate=60"]]'
)


RpcCall = Callable[[str, Sequence[str] | None], Mapping[str, Any]]


@dataclass(frozen=True)
class CoreLightningSnapshot:
    """Curated RPC payload bundle.

    Raw RPC responses NEVER reach this object. ``fetch_core_lightning_snapshot``
    passes each row through a per-resource ``_sanitize_*`` helper at the
    transport boundary, dropping every Tier-1 sensitive field (preimages,
    payment_secrets, bolt11 blobs, onion hops, route hints from received
    invoices, failure_source_pubkey / erring_node) before constructing the
    snapshot. The reshape helpers below operate exclusively on these typed
    collections, so a future caller, debug dump, or accidental persistence
    path cannot leak fields the snapshot never held in the first place.
    """

    node_id: str
    node_alias: str
    network: str
    implementation_version: str | None
    block_height: int | None
    peer_count: int
    channels: tuple[Mapping[str, Any], ...]
    funds_outputs: tuple[Mapping[str, Any], ...]
    forwards: tuple[Mapping[str, Any], ...]
    pays: tuple[Mapping[str, Any], ...]
    invoices: tuple[Mapping[str, Any], ...]
    income_events: tuple[Mapping[str, Any], ...]
    balance_accounts: tuple[Mapping[str, Any], ...]
    errors: Mapping[str, str]
    # bkpr-listaccountevents rows (channel_open/channel_close carry the on-chain
    # funding/closing txids). Defaulted so older constructions/tests still work.
    account_events: tuple[Mapping[str, Any], ...] = ()


# --- Helpers ---------------------------------------------------------------


def _json_dumps(value: Any) -> str:
    return json.dumps(json_ready(value), sort_keys=True, separators=(",", ":"))


def _stable_hash(parts: Sequence[Any]) -> str:
    payload = _json_dumps(list(parts)).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def _parse_msat(value: Any) -> int:
    if value in (None, ""):
        return 0
    if isinstance(value, bool):
        return 0
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        return int(value)
    if isinstance(value, Mapping):
        for key in ("msat", "millisatoshis", "millisatoshi"):
            if key in value:
                return _parse_msat(value[key])
        return 0
    text = str(value).strip().lower().replace(",", "")
    if not text:
        return 0
    for suffix, factor in (("msat", 1), ("sat", 1000), ("btc", 100_000_000_000)):
        if text.endswith(suffix):
            number = text[: -len(suffix)].strip()
            try:
                return int(float(number) * factor)
            except ValueError as exc:
                raise AppError(f"Invalid Core Lightning msat value '{value}'") from exc
    try:
        return int(text)
    except ValueError as exc:
        raise AppError(f"Invalid Core Lightning msat value '{value}'") from exc


def _msat_to_sat(value: int) -> int:
    if value >= 0:
        return value // 1000
    # Symmetric rounding toward zero for outflows.
    return -((-value) // 1000)


def _timestamp(value: Any) -> str:
    if value in (None, "", 0, "0"):
        return UNKNOWN_OCCURRED_AT
    try:
        return timestamp_to_iso(int(float(value)))
    except (TypeError, ValueError):
        return UNKNOWN_OCCURRED_AT


def _normalize_payment_hash(value: Any) -> str | None:
    text = str_or_none(value)
    if text is None:
        return None
    text = text.lower()
    if len(text) != 64:
        return None
    try:
        bytes.fromhex(text)
    except ValueError:
        return None
    return text


def _is_private_channel(channel: Mapping[str, Any]) -> bool:
    for key in ("private", "is_private"):
        value = channel.get(key)
        if isinstance(value, bool):
            return value
    return False


# --- RPC transport ---------------------------------------------------------


def _resolve_lightning_cli(backend: Mapping[str, Any]) -> str:
    return backend_value(backend, "lightning_cli") or "lightning-cli"


def _base_command(backend: Mapping[str, Any], method: str, args: Sequence[str] | None) -> list[str]:
    binary = _resolve_lightning_cli(backend)
    command = [binary, "--json", "--raw"]
    lightning_dir = backend_value(backend, "lightning_dir")
    if lightning_dir:
        command.append(f"--lightning-dir={lightning_dir}")
    rpc_file = backend_value(backend, "rpc_file")
    if rpc_file:
        command.append(f"--rpc-file={rpc_file}")
    network = backend_value(backend, "network")
    if network:
        command.append(f"--network={network}")
    command.append(method)
    command.extend(args or ())
    return command


def _commando_invocation(
    backend: Mapping[str, Any], base_command: Sequence[str]
) -> tuple[list[str], dict[str, str], str | None, str | None]:
    """Decide whether the call needs commando and how to pass the rune."""
    peer_id = backend_value(backend, "commando_peer_id")
    rune = backend_value(backend, "token")
    wants_commando = (
        bool(peer_id)
        or bool(rune)
        or str(backend.get("url") or "").lower().startswith("cln://commando")
    )
    if not wants_commando:
        return list(base_command), {}, None, None
    if not (peer_id and rune):
        raise AppError(
            "Core Lightning commando sync requires both --commando-peer-id and a rune token",
            code="validation",
            hint="Pipe a restricted rune through --token-stdin or --token-fd FD.",
        )
    command = list(base_command)
    insert_at = 3  # after [binary, "--json", "--raw"]
    command[insert_at:insert_at] = [
        f"--commando-peer={peer_id}",
        f"--commando-rune={rune}",
    ]
    return command, {}, None, "<commando rune redacted>"


def _redacted_command(command: Sequence[str]) -> list[str]:
    return [
        "<commando rune redacted>"
        if (
            "${LIGHTNING_RUNE}" in part
            or part.startswith("--commando-rune=")
            or part.startswith("--commando=")
        )
        else part
        for part in command
    ]


def call_core_lightning(
    backend: Mapping[str, Any],
    method: str,
    args: Sequence[str] | None = None,
) -> Mapping[str, Any]:
    """Invoke ``lightning-cli`` for ``method``.

    The method must appear in :data:`CLN_ALLOWED_METHODS`; pay/close/withdraw
    style requests are rejected at this boundary even if the rune would
    technically allow them.
    """
    if method not in CLN_ALLOWED_METHODS:
        raise AppError(
            f"Core Lightning sync refused unsupported RPC method '{method}'",
            code="validation",
            hint="Only read-only list/get/bookkeeper methods are allowed.",
        )
    base = _base_command(backend, method, args)
    command, env_extra, _stdin, _redacted = _commando_invocation(backend, base)
    env = dict(os.environ)
    env.update(env_extra)
    try:
        completed = subprocess.run(
            command,
            check=False,
            capture_output=True,
            text=True,
            timeout=backend_timeout(backend, default=30),
            env=env,
        )
    except FileNotFoundError as exc:
        raise AppError(
            "Core Lightning CLI executable was not found",
            code="dependency_missing",
            hint=(
                "Install Core Lightning or set backend config lightning_cli to"
                " the lightning-cli path."
            ),
            details={"command": command[0]},
            retryable=True,
        ) from exc
    except subprocess.TimeoutExpired as exc:
        raise AppError(
            f"Core Lightning RPC method '{method}' timed out",
            code="timeout",
            details={"method": method, "command": _redacted_command(command)},
            retryable=True,
        ) from exc
    if completed.returncode != 0:
        stderr = (completed.stderr or completed.stdout or "").strip()
        raise AppError(
            f"Core Lightning RPC method '{method}' failed",
            code="sync_error",
            hint=(
                "Check that the backend points at a running CLN node and that"
                " the rune allows this read method."
            ),
            details={
                "method": method,
                "command": _redacted_command(command),
                "stderr": stderr[-800:],
            },
            retryable=True,
        )
    try:
        payload = json.loads(completed.stdout or "{}")
    except json.JSONDecodeError as exc:
        raise AppError(
            f"Core Lightning RPC method '{method}' did not return JSON",
            code="sync_error",
            details={"method": method, "command": _redacted_command(command)},
            retryable=True,
        ) from exc
    return payload if isinstance(payload, Mapping) else {"result": payload}


_PEER_CHANNEL_KEEP: tuple[str, ...] = (
    "peer_id",
    "peer_alias",
    "alias",
    "peer_connected",
    "private",
    "is_private",
    "channel_id",
    "short_channel_id",
    "state",
    "status",
    "total_msat",
    "amount_msat",
    "funding_msat",
    "to_us_msat",
    "our_amount_msat",
    "their_amount_msat",
    "opener",
    "funder",
    "fee_base_msat",
    "fee_proportional_millionths",
    "opened_at",
    "closed_at",
    "funding",
    "funding_txid",
    "funding_outnum",
    "funding_outpoint",
)

_LISTFUNDS_OUTPUT_KEEP: tuple[str, ...] = (
    "status",
    "reserved_to_block",
    "amount_msat",
)

_FORWARD_KEEP: tuple[str, ...] = (
    "in_channel",
    "out_channel",
    "fee_msat",
    "in_msat",
    "out_msat",
    "status",
    "received_time",
    "resolved_time",
)

_PAY_KEEP: tuple[str, ...] = (
    "payment_hash",
    "amount_msat",
    "amount_sent_msat",
    "status",
    "created_at",
    "completed_at",
    "destination",
    "rebalance",
)

_INVOICE_KEEP: tuple[str, ...] = (
    "label",
    "payment_hash",
    "amount_msat",
    "amount_received_msat",
    "status",
    "paid_at",
    "created_at",
    "description",
)

_INCOME_KEEP: tuple[str, ...] = (
    "account",
    "tag",
    "credit_msat",
    "debit_msat",
    "currency",
    "timestamp",
    "payment_id",
    "description",
    "txid",
)

_BALANCE_ACCOUNT_KEEP: tuple[str, ...] = (
    "account",
    "peer_id",
    "balances",
    "account_closed",
)


def _pick(row: Mapping[str, Any], keys: Sequence[str]) -> dict[str, Any]:
    """Return a dict containing only ``keys`` that are present in ``row``."""
    return {key: row[key] for key in keys if key in row}


def _sanitize_peer_channel(row: Mapping[str, Any]) -> dict[str, Any]:
    """Curate one listpeerchannels row.

    Drops every Tier-1 sensitive field by virtue of the allowlist:
    ``htlcs`` (preimages), ``last_routing_failure``, peer announcement
    fragments, etc. never enter the snapshot.
    """
    curated = _pick(row, _PEER_CHANNEL_KEEP)
    funding = curated.get("funding")
    if isinstance(funding, Mapping):
        curated["funding"] = _pick(funding, ("txid", "funding_txid", "output", "outnum"))
    return curated


def _sanitize_listfunds_output(row: Mapping[str, Any]) -> dict[str, Any]:
    """Curate one listfunds output.

    Drops on-chain ``address`` / ``redeemscript`` / wallet ``output`` keys to
    avoid leaking the operator's L1 receive addresses through the snapshot.
    """
    return _pick(row, _LISTFUNDS_OUTPUT_KEEP)


def _sanitize_forward(row: Mapping[str, Any]) -> dict[str, Any]:
    """Curate one listforwards row.

    Drops ``failcode`` / ``failreason`` / ``erring_node`` / payment_hash and
    every other forward field that would let a future caller correlate "X
    paid Y through me" patterns.
    """
    return _pick(row, _FORWARD_KEEP)


def _sanitize_pay(row: Mapping[str, Any]) -> dict[str, Any]:
    """Curate one listpays row.

    Drops ``preimage`` / ``bolt11`` / ``route`` / ``erroronion`` / partial-id
    fields. Keeps ``payment_hash`` (needed for dedupe against bookkeeper
    rebalance_fee events).
    """
    return _pick(row, _PAY_KEEP)


def _sanitize_invoice(row: Mapping[str, Any]) -> dict[str, Any]:
    """Curate one listinvoices row.

    Drops ``payment_preimage`` / ``payment_secret`` / ``bolt11`` / ``routes``
    (route hints would leak third-party private-channel peers).
    """
    return _pick(row, _INVOICE_KEEP)


def _sanitize_income_event(row: Mapping[str, Any]) -> dict[str, Any]:
    """Curate one bkpr-listincome row.

    Drops free-text ``origin`` blobs that bookkeeper may attach to certain
    income categories. Keeps ``txid`` (used to filter out L1 collisions) and
    ``payment_id`` (used as the cross-resource payment_hash for dedupe).
    """
    return _pick(row, _INCOME_KEEP)


_ACCOUNT_EVENT_KEEP: tuple[str, ...] = (
    "account",
    "type",
    "tag",
    "txid",
    "outpoint",
    "timestamp",
)


def _sanitize_account_event(row: Mapping[str, Any]) -> dict[str, Any]:
    """Curate one bkpr-listaccountevents row.

    We only use ``channel_open`` / ``channel_close`` rows to harvest the
    on-chain funding/closing txids for channel-lifecycle netting, so keep the
    account, tag and txid and drop amounts / descriptions / blockheights.
    """
    return _pick(row, _ACCOUNT_EVENT_KEEP)


def _sanitize_balance_account(row: Mapping[str, Any]) -> dict[str, Any]:
    """Curate one bkpr-listbalances account entry.

    Trims the inner ``balances`` list to ``balance_msat`` only and drops the
    rest of the bookkeeper-internal metadata.
    """
    curated = _pick(row, _BALANCE_ACCOUNT_KEEP)
    balances = curated.get("balances")
    if isinstance(balances, list):
        curated["balances"] = [
            _pick(entry, ("balance_msat", "coin_type"))
            for entry in balances
            if isinstance(entry, Mapping)
        ]
    return curated


def _sanitize_list(
    payload: Mapping[str, Any] | None,
    field: str,
    sanitizer: Callable[[Mapping[str, Any]], Mapping[str, Any]],
) -> tuple[Mapping[str, Any], ...]:
    if not isinstance(payload, Mapping):
        return ()
    rows = payload.get(field)
    if not isinstance(rows, list):
        return ()
    return tuple(sanitizer(row) for row in rows if isinstance(row, Mapping))


def fetch_core_lightning_snapshot(
    backend: Mapping[str, Any],
    *,
    rpc_call: RpcCall | None = None,
) -> CoreLightningSnapshot:
    """Drive ``call_core_lightning`` once per allowed method.

    Every raw RPC response is passed through a per-resource ``_sanitize_*``
    helper before the snapshot is constructed, so the in-memory
    :class:`CoreLightningSnapshot` cannot carry preimages, bolt11 strings,
    onion routes, payment_secrets, route hints from received invoices, or
    ``erring_node`` markers. The reshape helpers below operate exclusively on
    these curated typed collections.
    """
    caller = rpc_call or (lambda method, args=None: call_core_lightning(backend, method, args))

    def _safe_call(method: str) -> Mapping[str, Any] | None:
        try:
            return caller(method, None)
        except AppError as exc:
            errors[method] = str(exc)
            return None

    errors: dict[str, str] = {}
    info_raw = caller("getinfo", None)
    info = info_raw if isinstance(info_raw, Mapping) else {}
    node_id = str_or_none(info.get("id")) or ""
    node_alias = str_or_none(info.get("alias")) or ""
    network = str_or_none(info.get("network")) or ""
    implementation_version = str_or_none(info.get("version"))
    block_height_raw = info.get("blockheight")
    block_height = int(block_height_raw) if isinstance(block_height_raw, (int, float)) else None
    peer_count_raw = info.get("num_peers")
    peer_count = int(peer_count_raw) if isinstance(peer_count_raw, (int, float)) else 0

    funds_payload = _safe_call("listfunds")
    channels_payload = _safe_call("listpeerchannels")
    forwards_payload = _safe_call("listforwards")
    pays_payload = _safe_call("listpays")
    invoices_payload = _safe_call("listinvoices")
    income_payload = _safe_call("bkpr-listincome")
    balances_payload = _safe_call("bkpr-listbalances")
    account_events_payload = _safe_call("bkpr-listaccountevents")

    return CoreLightningSnapshot(
        node_id=node_id,
        node_alias=node_alias,
        network=network,
        implementation_version=implementation_version,
        block_height=block_height,
        peer_count=peer_count,
        channels=_sanitize_list(channels_payload, "channels", _sanitize_peer_channel),
        funds_outputs=_sanitize_list(funds_payload, "outputs", _sanitize_listfunds_output),
        forwards=_sanitize_list(forwards_payload, "forwards", _sanitize_forward),
        pays=_sanitize_list(pays_payload, "pays", _sanitize_pay),
        invoices=_sanitize_list(invoices_payload, "invoices", _sanitize_invoice),
        income_events=_sanitize_list(income_payload, "income_events", _sanitize_income_event),
        balance_accounts=_sanitize_list(balances_payload, "accounts", _sanitize_balance_account),
        account_events=_sanitize_list(account_events_payload, "events", _sanitize_account_event),
        errors=errors,
    )


# --- Reshape: RPC -> NodeSnapshot ------------------------------------------


_CHANNEL_STATE_MAP: dict[str, NodeChannelState] = {
    "channeld_normal": "active",
    "channeld_awaiting_lockin": "pending_open",
    "channeld_shutting_down": "pending_close",
    "closingd_sigexchange": "pending_close",
    "closingd_complete": "closed",
    "onchaind_their_unilateral": "force_closed",
    "onchaind_our_unilateral": "force_closed",
    "fundingd": "pending_open",
    "openingd": "pending_open",
    "dualopend_open_init": "pending_open",
    "dualopend_awaiting_lockin": "pending_open",
    "closed": "closed",
}


def _coerce_channel_state(value: Any, connected: bool | None = None) -> NodeChannelState:
    text = (str_or_none(value) or "").lower()
    if not text:
        return "inactive"
    if "breach" in text:
        return "force_closed"
    if text in _CHANNEL_STATE_MAP:
        return _CHANNEL_STATE_MAP[text]
    if "onchain" in text:
        return "force_closed"
    if "shutdown" in text or "closing" in text:
        return "pending_close"
    if "await" in text or "open" in text:
        return "pending_open"
    if "normal" in text and connected is False:
        return "inactive"
    if "normal" in text:
        return "active"
    if "close" in text:
        return "closed"
    return "inactive"


def _channel_close_kind(state_raw: Any, state: NodeChannelState) -> str | None:
    text = (str_or_none(state_raw) or "").lower()
    if "breach" in text:
        return "breach"
    if state == "force_closed":
        return "force"
    if state == "closed":
        return "cooperative"
    return None


def _channel_short_id(channel: Mapping[str, Any]) -> str | None:
    for key in ("short_channel_id", "alias", "channel_id"):
        value = str_or_none(channel.get(key))
        if value:
            return value
    return None


def _channel_funding_outpoint(channel: Mapping[str, Any]) -> str | None:
    funding = channel.get("funding")
    if isinstance(funding, Mapping):
        txid = str_or_none(funding.get("txid") or funding.get("funding_txid"))
        outnum = funding.get("output") or funding.get("outnum")
        if txid and outnum is not None:
            return f"{txid}:{outnum}"
    txid = str_or_none(channel.get("funding_txid"))
    outnum = channel.get("funding_outnum")
    if txid and outnum is not None:
        return f"{txid}:{outnum}"
    return str_or_none(channel.get("funding_outpoint"))


def _node_channel(channel: Mapping[str, Any], peer_alias_map: Mapping[str, str]) -> NodeChannel:
    is_private = _is_private_channel(channel)
    raw_peer = str_or_none(channel.get("peer_id"))
    peer_alias = peer_alias_map.get(raw_peer or "")
    if not peer_alias:
        peer_alias = "private peer" if is_private else (raw_peer or "")
    # Opsec: drop peer pubkey for private channels by default.
    peer_pubkey: str | None = None if is_private else raw_peer
    capacity_msat = _parse_msat(
        channel.get("total_msat") or channel.get("amount_msat") or channel.get("funding_msat")
    )
    to_us_msat = _parse_msat(channel.get("to_us_msat") or channel.get("our_amount_msat"))
    if capacity_msat <= 0:
        capacity_msat = to_us_msat + _parse_msat(channel.get("their_amount_msat"))
    remote_msat = max(capacity_msat - to_us_msat, 0)
    state_raw = channel.get("state") or channel.get("status")
    state = _coerce_channel_state(state_raw, connected=channel.get("peer_connected"))
    channel_id = (
        str_or_none(channel.get("channel_id"))
        or str_or_none(channel.get("short_channel_id"))
        or _channel_funding_outpoint(channel)
        or ""
    )
    return NodeChannel(
        id=channel_id,
        peer_alias=peer_alias or "",
        peer_pubkey=peer_pubkey,
        capacity_sat=_msat_to_sat(capacity_msat),
        local_balance_sat=_msat_to_sat(to_us_msat),
        remote_balance_sat=_msat_to_sat(remote_msat),
        state=state,
        is_private=is_private,
        is_initiator=bool(channel.get("opener") == "local" or channel.get("funder") == "LOCAL"),
        short_channel_id=_channel_short_id(channel),
        funding_outpoint=_channel_funding_outpoint(channel),
        base_fee_msat=_parse_msat(channel.get("fee_base_msat")) or None,
        fee_rate_ppm=int(channel.get("fee_proportional_millionths") or 0) or None,
        opened_at=_timestamp(channel.get("opened_at")) if channel.get("opened_at") else None,
        closed_at=_timestamp(channel.get("closed_at")) if channel.get("closed_at") else None,
        close_kind=_channel_close_kind(state_raw, state),
    )


def _build_peer_alias_map(snapshot: CoreLightningSnapshot) -> dict[str, str]:
    aliases: dict[str, str] = {}
    # Aliases come most reliably from listpeerchannels' per-peer fields or
    # from listnodes (which we don't call). Fall back to short ids.
    for channel in snapshot.channels:
        peer_id = str_or_none(channel.get("peer_id"))
        alias = str_or_none(channel.get("peer_alias") or channel.get("alias"))
        if peer_id and alias:
            aliases[peer_id] = alias
    return aliases


def _onchain_balance_msat(snapshot: CoreLightningSnapshot) -> int:
    total = 0
    for entry in snapshot.funds_outputs:
        if str_or_none(entry.get("status")) not in {"confirmed", "unconfirmed", None}:
            continue
        if str_or_none(entry.get("reserved_to_block")):
            continue
        total += _parse_msat(entry.get("amount_msat"))
    return total


def _forward_status(value: Any) -> NodeForwardStatus:
    text = (str_or_none(value) or "").lower()
    if text in {"settled", "complete", "completed"}:
        return "settled"
    if text in {"offered", "pending"}:
        return "offered"
    return "failed"


def _build_forwards(
    snapshot: CoreLightningSnapshot, peer_alias_map: Mapping[str, str]
) -> tuple[NodeForward, ...]:
    results: list[NodeForward] = []
    for forward in snapshot.forwards:
        in_channel = str_or_none(forward.get("in_channel"))
        out_channel = str_or_none(forward.get("out_channel"))
        results.append(
            NodeForward(
                id=_stable_hash(
                    (
                        "fw",
                        in_channel,
                        out_channel,
                        forward.get("received_time"),
                        forward.get("resolved_time"),
                        forward.get("in_msat"),
                        forward.get("out_msat"),
                        forward.get("status"),
                    )
                ),
                occurred_at=_timestamp(
                    forward.get("resolved_time") or forward.get("received_time")
                ),
                in_peer_alias=peer_alias_map.get(in_channel or "") or (in_channel or ""),
                out_peer_alias=peer_alias_map.get(out_channel or "") or (out_channel or ""),
                amount_in_msat=_parse_msat(forward.get("in_msat")),
                amount_out_msat=_parse_msat(forward.get("out_msat")),
                fee_msat=_parse_msat(forward.get("fee_msat")),
                status=_forward_status(forward.get("status")),
                in_short_channel_id=in_channel,
                out_short_channel_id=out_channel,
                # Opsec: drop failure_reason / erring_node — failure
                # diagnosis has no tax value and would leak which nodes
                # the operator tried.
                failure_reason=None,
            )
        )
    return tuple(results)


def _routing_summary(
    snapshot: CoreLightningSnapshot, window_days: int
) -> NodeRoutingSnapshot | None:
    settled_forwards = [
        row for row in snapshot.forwards if _forward_status(row.get("status")) == "settled"
    ]
    routing_revenue_msat = sum(_parse_msat(row.get("fee_msat")) for row in settled_forwards)
    forward_count = len(settled_forwards)

    # Cost split between user payments and rebalances:
    # - Payment fees come from listpays (the operator-visible cost).
    # - Rebalance fees come from bkpr-listincome `rebalance_fee` events
    #   (the bookkeeper's canonical view). We deliberately do NOT add the
    #   fee from a rebalance-tagged listpays row because that same fee will
    #   already appear as a bookkeeper rebalance_fee event for the same
    #   payment_hash. Adding both double-counts (M-1).
    # - Rebalance _count_ stays driven by listpays because the bookkeeper
    #   rebalance_fee event collapses multi-part rebalances under one fee
    #   row, while listpays carries one row per attempt.
    payment_cost_msat = 0
    payment_count = 0
    rebalance_count = 0
    for pay in snapshot.pays:
        if str_or_none(pay.get("status")) not in {"complete", "completed", "paid"}:
            continue
        if pay.get("rebalance"):
            rebalance_count += 1
            continue
        amount_sent = _parse_msat(pay.get("amount_sent_msat"))
        amount = _parse_msat(pay.get("amount_msat"))
        fee = max(0, amount_sent - amount)
        payment_cost_msat += fee
        payment_count += 1

    rebalance_cost_msat = 0
    onchain_cost_msat = 0
    for event in snapshot.income_events:
        tag = (str_or_none(event.get("tag")) or "").lower()
        if tag == "rebalance_fee":
            # bookkeeper-canonical view: ``debit_msat`` IS the fee, not
            # principal + fee.
            rebalance_cost_msat += _parse_msat(
                event.get("debit_msat") or event.get("credit_msat")
            )
        elif tag in {"onchain_fee", "onchain_fees"}:
            onchain_cost_msat += _parse_msat(
                event.get("debit_msat") or event.get("credit_msat")
            )

    if not (
        forward_count
        or payment_count
        or rebalance_count
        or routing_revenue_msat
        or payment_cost_msat
        or rebalance_cost_msat
        or onchain_cost_msat
    ):
        return None
    net_msat = routing_revenue_msat - payment_cost_msat - rebalance_cost_msat - onchain_cost_msat
    return NodeRoutingSnapshot(
        window_label=f"Last {window_days} days",
        routing_revenue_sat=_msat_to_sat(routing_revenue_msat),
        payment_cost_sat=_msat_to_sat(payment_cost_msat),
        rebalance_cost_sat=_msat_to_sat(rebalance_cost_msat),
        onchain_cost_sat=_msat_to_sat(onchain_cost_msat),
        net_profit_sat=_msat_to_sat(net_msat),
        forward_count=forward_count,
        payment_count=payment_count,
        rebalance_count=rebalance_count,
    )


def _per_channel_routing(snapshot: CoreLightningSnapshot) -> dict[str, int]:
    earned_per_channel: dict[str, int] = {}
    for row in snapshot.forwards:
        if _forward_status(row.get("status")) != "settled":
            continue
        out_channel = str_or_none(row.get("out_channel")) or ""
        if not out_channel:
            continue
        earned_per_channel[out_channel] = earned_per_channel.get(out_channel, 0) + _parse_msat(
            row.get("fee_msat")
        )
    return {k: _msat_to_sat(v) for k, v in earned_per_channel.items()}


def _invoice_counts(snapshot: CoreLightningSnapshot) -> tuple[int, int, int]:
    invoice_count = len(snapshot.invoices)
    paid = 0
    expired = 0
    for invoice in snapshot.invoices:
        status = (str_or_none(invoice.get("status")) or "").lower()
        if status == "paid":
            paid += 1
        elif status == "expired":
            expired += 1
    return invoice_count, paid, expired


def _payment_counts(snapshot: CoreLightningSnapshot) -> tuple[int, int, int]:
    payment_count = len(snapshot.pays)
    complete = 0
    failed = 0
    for pay in snapshot.pays:
        status = (str_or_none(pay.get("status")) or "").lower()
        if status in {"complete", "completed", "paid"}:
            complete += 1
        elif status in {"failed", "error"}:
            failed += 1
    return payment_count, complete, failed


def build_node_snapshot(
    snapshot: CoreLightningSnapshot, *, window_days: int
) -> NodeSnapshot:
    peer_alias_map = _build_peer_alias_map(snapshot)
    channels = tuple(_node_channel(row, peer_alias_map) for row in snapshot.channels)
    earned_per_channel = _per_channel_routing(snapshot)
    enriched_channels = tuple(
        (
            channel
            if channel.short_channel_id not in earned_per_channel
            else _replace_channel(channel, earned_routing_sat=earned_per_channel[channel.short_channel_id])
        )
        for channel in channels
    )
    closed_channels = tuple(
        channel for channel in enriched_channels if channel.state in {"closed", "force_closed"}
    )
    open_channels = tuple(
        channel for channel in enriched_channels if channel.state not in {"closed", "force_closed"}
    )

    total_local_msat = sum(
        _parse_msat({"msat": channel.local_balance_sat * 1000}) for channel in open_channels
    )
    total_remote_msat = sum(
        _parse_msat({"msat": channel.remote_balance_sat * 1000}) for channel in open_channels
    )
    total_capacity_msat = sum(
        _parse_msat({"msat": channel.capacity_sat * 1000}) for channel in open_channels
    )

    onchain_balance_msat = _onchain_balance_msat(snapshot)

    routing = _routing_summary(snapshot, window_days)
    forwards = _build_forwards(snapshot, peer_alias_map)
    invoice_count, paid_invoice_count, expired_invoice_count = _invoice_counts(snapshot)
    payment_count, completed_payment_count, failed_payment_count = _payment_counts(snapshot)

    return NodeSnapshot(
        alias=snapshot.node_alias,
        pubkey=snapshot.node_id,
        network=snapshot.network or "mainnet",
        peer_count=snapshot.peer_count,
        onchain_balance_sat=_msat_to_sat(onchain_balance_msat),
        total_local_balance_sat=_msat_to_sat(total_local_msat),
        total_remote_balance_sat=_msat_to_sat(total_remote_msat),
        total_capacity_sat=_msat_to_sat(total_capacity_msat),
        channels=open_channels,
        closed_channels=closed_channels,
        implementation_version=snapshot.implementation_version,
        block_height=snapshot.block_height,
        invoice_count=invoice_count,
        paid_invoice_count=paid_invoice_count,
        expired_invoice_count=expired_invoice_count,
        payment_count=payment_count,
        completed_payment_count=completed_payment_count,
        failed_payment_count=failed_payment_count,
        routing=routing,
        forwards=forwards,
    )


def _replace_channel(channel: NodeChannel, **kwargs: Any) -> NodeChannel:
    from dataclasses import replace as _dc_replace

    return _dc_replace(channel, **kwargs)


# --- LightningAdapter ------------------------------------------------------


class CoreLightningAdapter:
    """Scaffold-compatible Core Lightning adapter."""

    kind = "coreln"
    capabilities = LightningCapabilities(
        node_snapshot=True,
        routing_profitability=True,
        channel_balances=True,
        channel_lifecycle=True,
        forward_events=True,
        invoice_activity=True,
        payment_activity=True,
        onchain_balance=True,
    )

    def fetch_node_snapshot(
        self,
        connection: dict[str, Any],
        backend: dict[str, Any] | None,
        *,
        window_days: int = 30,
    ) -> NodeSnapshot:
        if backend is None:
            raise AppError(
                "Core Lightning connection requires a backend configuration",
                code="validation",
                hint=(
                    "Create a coreln backend (`kassiber backends create cln"
                    " --kind coreln ...`) and reference it from the wallet."
                ),
                retryable=False,
            )
        snapshot = fetch_core_lightning_snapshot(backend)
        return build_node_snapshot(snapshot, window_days=window_days)


# --- Reshape: RPC -> persisted (curated) records ---------------------------
# Persistence is opt-in via ``sync_core_lightning_wallet`` below. The records
# stored on disk are reshaped Tier-2 aggregates plus invoice-derived income
# rows that become wallet transactions; raw RPC payloads are NEVER persisted.


def _aggregate_forwards(
    snapshot: CoreLightningSnapshot, peer_alias_map: Mapping[str, str]
) -> list[dict[str, Any]]:
    """Collapse listforwards into one row per (day, out_channel).

    Closes the double-count P1 (bkpr-listincome ``routed`` events) and bounds
    the privacy surface: the DB never holds a complete log of "X paid Y
    through me" patterns.
    """
    buckets: dict[tuple[str, str], MutableMapping[str, Any]] = {}
    for row in snapshot.forwards:
        if _forward_status(row.get("status")) != "settled":
            continue
        occurred = _timestamp(row.get("resolved_time") or row.get("received_time"))
        day = occurred[:10] if occurred and occurred != UNKNOWN_OCCURRED_AT else occurred
        out_channel = str_or_none(row.get("out_channel")) or "unknown"
        key = (day, out_channel)
        bucket = buckets.setdefault(
            key,
            {
                "occurred_at": f"{day}T00:00:00Z" if day != UNKNOWN_OCCURRED_AT else occurred,
                "channel_id": out_channel,
                "fee_msat": 0,
                "amount_msat": 0,
                "forward_count": 0,
            },
        )
        bucket["fee_msat"] += _parse_msat(row.get("fee_msat"))
        bucket["amount_msat"] += _parse_msat(row.get("out_msat"))
        bucket["forward_count"] += 1
    records: list[dict[str, Any]] = []
    for (day, channel_id), bucket in sorted(buckets.items()):
        records.append(
            {
                "record_type": "forward_day",
                "external_id": _stable_hash(("fw_day", day, channel_id)),
                "occurred_at": bucket["occurred_at"],
                "account": channel_id,
                "peer_id": None,  # Opsec: drop peer pubkey on forwards.
                "channel_id": channel_id,
                "direction": "inbound",
                "amount_msat": bucket["amount_msat"],
                "fee_msat": bucket["fee_msat"],
                "tag": "routing_fee",
                "status": "settled",
                "currency": "bc",
                "payment_hash": None,
                "txid": None,
                "outpoint": None,
            }
        )
    return records


def _invoice_record(invoice: Mapping[str, Any]) -> dict[str, Any] | None:
    # Opsec: drop payment_preimage, bolt11, payment_secret, route_hints.
    if str_or_none(invoice.get("status")) != "paid":
        return None
    payment_hash = _normalize_payment_hash(invoice.get("payment_hash"))
    amount_msat = _parse_msat(
        invoice.get("amount_received_msat") or invoice.get("amount_msat")
    )
    if amount_msat <= 0:
        return None
    label = str_or_none(invoice.get("label"))
    description = str_or_none(invoice.get("description"))
    external_id = payment_hash or label or _stable_hash(("invoice", invoice.get("paid_at")))
    return {
        "record_type": "invoice",
        "external_id": external_id,
        "occurred_at": _timestamp(invoice.get("paid_at") or invoice.get("created_at")),
        "account": None,
        "peer_id": None,
        "channel_id": None,
        "direction": "inbound",
        "amount_msat": amount_msat,
        "fee_msat": 0,
        "tag": "invoice",
        "status": description or label or "paid",
        "currency": "bc",
        "payment_hash": payment_hash,
        "txid": None,
        "outpoint": None,
    }


def _pay_record(pay: Mapping[str, Any]) -> dict[str, Any] | None:
    # Opsec: drop payment_preimage, bolt11, route hops. Keep
    # amount/fee/destination/payment_hash only.
    payment_hash = _normalize_payment_hash(pay.get("payment_hash"))
    amount_sent = _parse_msat(pay.get("amount_sent_msat"))
    amount = _parse_msat(pay.get("amount_msat"))
    fee_msat = max(0, amount_sent - amount)
    status = (str_or_none(pay.get("status")) or "").lower()
    if status not in {"complete", "completed", "paid"}:
        return None
    return {
        "record_type": "pay",
        "external_id": payment_hash or _stable_hash(("pay", pay.get("created_at"), amount_sent)),
        "occurred_at": _timestamp(pay.get("completed_at") or pay.get("created_at")),
        "account": None,
        "peer_id": str_or_none(pay.get("destination")),
        "channel_id": None,
        "direction": "outbound",
        "amount_msat": amount or amount_sent,
        "fee_msat": fee_msat,
        "tag": "rebalance" if pay.get("rebalance") else "payment",
        "status": status,
        "currency": "bc",
        "payment_hash": payment_hash,
        "txid": None,
        "outpoint": None,
    }


def _income_invoice_record(event: Mapping[str, Any]) -> dict[str, Any] | None:
    """Map a bkpr-listincome row to a Tier-2 income row IFF it is an invoice.

    P1 fix #1: original implementation imported every income/outbound row,
    including ``routed`` events, double-counting routing fees against
    bkpr-listincome AND listforwards. Restrict to ``tag == "invoice"`` and
    drop any row with a non-null ``txid`` to avoid colliding with the
    operator's L1 wallet sync.
    """
    tag = (str_or_none(event.get("tag")) or "").lower()
    if tag != "invoice":
        return None
    if str_or_none(event.get("txid")):
        return None
    credit = _parse_msat(event.get("credit_msat"))
    debit = _parse_msat(event.get("debit_msat"))
    net = credit - debit
    if net <= 0:
        return None
    payment_hash = _normalize_payment_hash(event.get("payment_id"))
    external_id = _stable_hash(
        ("income_invoice", event.get("account"), event.get("timestamp"), payment_hash, credit)
    )
    return {
        "record_type": "income",
        "external_id": external_id,
        "occurred_at": _timestamp(event.get("timestamp")),
        "account": str_or_none(event.get("account")),
        "peer_id": None,
        "channel_id": str_or_none(event.get("account")),
        "direction": "inbound",
        "amount_msat": net,
        "fee_msat": 0,
        "tag": "invoice",
        "status": str_or_none(event.get("description")) or "invoice",
        "currency": str_or_none(event.get("currency")) or "bc",
        "payment_hash": payment_hash,
        "txid": None,
        "outpoint": None,
    }


def _balance_snapshot_records(
    snapshot: CoreLightningSnapshot, synced_at: str
) -> list[dict[str, Any]]:
    """Daily-bucketed balance snapshot.

    P1 fix #2: original implementation used ``synced_at`` (per-sync timestamp)
    in the hash, so every sync added fresh rows. We bucket on the date so a
    given day's snapshot UPDATEs rather than appends.
    """
    bucket_day = synced_at[:10] if synced_at and len(synced_at) >= 10 else synced_at
    records: list[dict[str, Any]] = []
    for account in snapshot.balance_accounts:
        balances = account.get("balances") if isinstance(account.get("balances"), list) else []
        total_msat = sum(
            _parse_msat(entry.get("balance_msat"))
            for entry in balances
            if isinstance(entry, Mapping)
        )
        account_name = str_or_none(account.get("account"))
        records.append(
            {
                "record_type": "balance_snapshot",
                "external_id": _stable_hash(("balance_day", bucket_day, account_name)),
                "occurred_at": f"{bucket_day}T00:00:00Z" if bucket_day else synced_at,
                "account": account_name,
                # Opsec: keep peer_id only for public-channel accounts. We
                # don't have a private flag at this layer, so drop entirely.
                "peer_id": None,
                "channel_id": account_name if account.get("peer_id") else None,
                "direction": "",
                "amount_msat": total_msat,
                "fee_msat": 0,
                "tag": "balance",
                "status": "closed" if account.get("account_closed") else "open",
                "currency": "",
                "payment_hash": None,
                "txid": None,
                "outpoint": None,
            }
        )
    return records


def _channel_record(
    tag: str, txid: str, account: str | None, amount_msat: int = 0
) -> dict[str, Any]:
    """A ``channel`` metadata record carrying one channel-lifecycle txid.

    ``tag`` is ``channel_open`` (funding) or ``channel_close`` (closing). These
    are NOT promoted to wallet transactions (``_record_to_import`` ignores
    them) — they let the tax engine recognize a separately-synced on-chain
    wallet's channel funding/close txs as non-taxable intra-node moves.
    ``channel_close`` records carry our settled balance leaving the channel
    (bookkeeper ``debit_msat``) so the engine can book the close fee — the gap
    between that balance and what the on-chain wallet actually received.
    """
    return {
        "record_type": "channel",
        "external_id": _stable_hash(("channel", tag, txid)),
        "occurred_at": UNKNOWN_OCCURRED_AT,
        "account": account,
        "peer_id": None,
        "channel_id": account,
        "direction": "",
        "amount_msat": int(amount_msat or 0),
        "fee_msat": 0,
        "tag": tag,
        "status": "",
        "currency": "bc",
        "payment_hash": None,
        "txid": txid,
        "outpoint": None,
        "raw_json": "{}",
    }


def _channel_lifecycle_records(snapshot: CoreLightningSnapshot) -> list[dict[str, Any]]:
    """Emit ``channel`` metadata records carrying channel funding/closing txids.

    Funding txids come from open channels' ``funding_outpoint`` AND bookkeeper
    ``channel_open`` events; closing txids come from bookkeeper ``channel_close``
    events (``listpeerchannels`` does not retain a channel once fully closed, so
    ``bkpr-listaccountevents`` is the reliable source for the closing tx).
    """
    open_txids: dict[str, str | None] = {}
    close_txids: dict[str, str | None] = {}
    close_balance_msat: dict[str, int] = {}
    for channel in snapshot.channels:
        outpoint = _channel_funding_outpoint(channel)
        funding_txid = outpoint.split(":", 1)[0] if outpoint else None
        if funding_txid:
            account = (
                str_or_none(channel.get("channel_id"))
                or _channel_short_id(channel)
                or funding_txid
            )
            open_txids.setdefault(funding_txid, account)
    for event in snapshot.account_events:
        tag = (str_or_none(event.get("tag")) or "").lower()
        outpoint = str_or_none(event.get("outpoint"))
        txid = str_or_none(event.get("txid")) or (
            outpoint.split(":", 1)[0] if outpoint else None
        )
        if not txid:
            continue
        account = str_or_none(event.get("account")) or txid
        if tag == "channel_open":
            open_txids.setdefault(txid, account)
        elif tag == "channel_close":
            close_txids.setdefault(txid, account)
            # Our settled balance leaving the channel account. The engine
            # books the gap between this and the on-chain receipt as the
            # close fee (commitment + sweep fees), instead of stranding it.
            balance = _parse_msat(event.get("debit_msat"))
            if balance > 0:
                close_balance_msat.setdefault(txid, balance)

    records: list[dict[str, Any]] = []
    for txid, account in sorted(open_txids.items()):
        records.append(_channel_record("channel_open", txid, account))
    for txid, account in sorted(close_txids.items()):
        records.append(
            _channel_record(
                "channel_close", txid, account, close_balance_msat.get(txid, 0)
            )
        )
    return records


def snapshot_records(snapshot: CoreLightningSnapshot, synced_at: str) -> list[dict[str, Any]]:
    """Reshape ``snapshot`` into the curated persistence rows.

    The list is intentionally narrow: aggregated forwards, paid invoices,
    completed pays, daily balance snapshots, channel funding/closing txids, and
    invoice-only bookkeeper income rows that become wallet transactions. No raw
    RPC payloads, no per-forward rows, no preimages, no bolt11 strings, no onion
    routes.
    """
    peer_alias_map = _build_peer_alias_map(snapshot)
    records: list[dict[str, Any]] = []
    records.extend(_aggregate_forwards(snapshot, peer_alias_map))
    for invoice in snapshot.invoices:
        record = _invoice_record(invoice)
        if record is not None:
            records.append(record)
    for pay in snapshot.pays:
        record = _pay_record(pay)
        if record is not None:
            records.append(record)
    for event in snapshot.income_events:
        record = _income_invoice_record(event)
        if record is not None:
            records.append(record)
    records.extend(_balance_snapshot_records(snapshot, synced_at))
    records.extend(_channel_lifecycle_records(snapshot))
    return records


# --- Persistence -----------------------------------------------------------


def _record_id(
    profile_id: str,
    wallet_id: str,
    backend_name: str,
    record_type: str,
    external_id: str,
) -> str:
    return _stable_hash((profile_id, wallet_id, backend_name, record_type, external_id))


def _upsert_lightning_record(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    backend: Mapping[str, Any],
    sync_id: str,
    node_id: str,
    record: Mapping[str, Any],
    timestamp: str,
) -> tuple[bool, bool]:
    record_id = _record_id(
        profile["id"],
        wallet["id"],
        backend["name"],
        record["record_type"],
        record["external_id"],
    )
    existing = conn.execute(
        "SELECT amount_msat, fee_msat, status FROM lightning_node_records WHERE id = ?",
        (record_id,),
    ).fetchone()
    conn.execute(
        """
        INSERT INTO lightning_node_records(
            id, workspace_id, profile_id, wallet_id, backend_name, node_id,
            record_type, external_id, occurred_at, account, peer_id, channel_id,
            direction, amount_msat, fee_msat, tag, status, currency, payment_hash,
            txid, outpoint, sync_id, raw_json, first_seen_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(profile_id, wallet_id, backend_name, record_type, external_id)
        DO UPDATE SET
            node_id = excluded.node_id,
            occurred_at = excluded.occurred_at,
            account = excluded.account,
            peer_id = excluded.peer_id,
            channel_id = excluded.channel_id,
            direction = excluded.direction,
            amount_msat = excluded.amount_msat,
            fee_msat = excluded.fee_msat,
            tag = excluded.tag,
            status = excluded.status,
            currency = excluded.currency,
            payment_hash = excluded.payment_hash,
            txid = excluded.txid,
            outpoint = excluded.outpoint,
            sync_id = excluded.sync_id,
            updated_at = excluded.updated_at
        """,
        (
            record_id,
            profile["workspace_id"],
            profile["id"],
            wallet["id"],
            backend["name"],
            node_id,
            record["record_type"],
            record["external_id"],
            record.get("occurred_at") or UNKNOWN_OCCURRED_AT,
            record.get("account"),
            record.get("peer_id"),
            record.get("channel_id"),
            record.get("direction"),
            int(record.get("amount_msat") or 0),
            int(record.get("fee_msat") or 0),
            record.get("tag"),
            record.get("status"),
            record.get("currency"),
            record.get("payment_hash"),
            record.get("txid"),
            record.get("outpoint"),
            sync_id,
            # raw_json kept empty by design — opsec policy bans full RPC
            # payloads on disk. The column stays for schema compatibility.
            "{}",
            timestamp,
            timestamp,
        ),
    )
    if existing is None:
        return True, False
    changed = (
        int(existing["amount_msat"] or 0) != int(record.get("amount_msat") or 0)
        or int(existing["fee_msat"] or 0) != int(record.get("fee_msat") or 0)
        or str(existing["status"] or "") != str(record.get("status") or "")
    )
    return False, changed


def _pay_to_import(record: Mapping[str, Any]) -> dict[str, Any] | None:
    """Promote a completed outbound pay row to a wallet transaction.

    Mirror of the invoice-income promotion for the outbound leg. Without this,
    Core Lightning spends never reach the ledger at all, and a payment between
    the operator's own nodes (e.g. CLN -> LND) leaves only the inbound invoice
    booked as income — a phantom taxable event with no offsetting outflow.

    The promoted row carries the ``payment_hash`` so
    :func:`kassiber.core.transfer_matching` can pair it with the matching
    inbound invoice on another owned wallet and reclassify the pair as an
    internal transfer, leaving only the routing fee as the taxable component.
    ``amount_msat`` here is the principal (``_pay_record`` already splits the
    routing fee into ``fee_msat``).
    """
    amount_msat = int(record.get("amount_msat") or 0)
    if amount_msat <= 0:
        return None
    fee_msat = int(record.get("fee_msat") or 0)
    payment_hash = record.get("payment_hash")
    return {
        "id": f"cln:pay:{record['external_id']}",
        "occurred_at": record.get("occurred_at") or UNKNOWN_OCCURRED_AT,
        "confirmed_at": record.get("occurred_at") or UNKNOWN_OCCURRED_AT,
        "direction": "outbound",
        "asset": "BTC",
        "amount": msat_to_btc(amount_msat),
        "fee": msat_to_btc(fee_msat),
        "kind": "cln_pay",
        "description": record.get("status") or "Core Lightning payment",
        "counterparty": record.get("peer_id"),
        "payment_hash": payment_hash,
        "payment_hash_source": "core_lightning" if payment_hash else None,
        "raw_json": "{}",
    }


def _record_to_import(record: Mapping[str, Any]) -> dict[str, Any] | None:
    """Promote invoice-income and completed-pay rows to wallet transactions.

    P1 fix #1: only ``tag=="invoice"`` income rows reach the income branch
    because :func:`_income_invoice_record` already filters them. The defensive
    checks here are kept so a future caller can't accidentally feed us a
    routed-event row that would double-count. Outbound ``pay`` rows are promoted
    via :func:`_pay_to_import` so own-node payments can pair by payment hash.
    """
    if record.get("record_type") == "pay" and record.get("direction") == "outbound":
        return _pay_to_import(record)
    if record.get("record_type") != "income":
        return None
    if (record.get("tag") or "").lower() != "invoice":
        return None
    if record.get("txid"):
        return None
    amount_msat = int(record.get("amount_msat") or 0)
    if amount_msat <= 0:
        return None
    if record.get("direction") != "inbound":
        return None
    return {
        "id": f"cln:income:{record['external_id']}",
        "occurred_at": record.get("occurred_at") or UNKNOWN_OCCURRED_AT,
        # Lightning invoice rows are promoted only after CLN reports them as
        # paid/settled. They do not have an L1 confirmation, but the desktop
        # status badge uses confirmed_at as the generic finality signal.
        "confirmed_at": record.get("occurred_at") or UNKNOWN_OCCURRED_AT,
        "direction": "inbound",
        "asset": "BTC",
        "amount": msat_to_btc(amount_msat),
        "fee": 0,
        "kind": "cln_invoice",
        "description": record.get("status") or "Core Lightning invoice",
        "counterparty": record.get("account"),
        "payment_hash": record.get("payment_hash"),
        "payment_hash_source": "core_lightning" if record.get("payment_hash") else None,
        "raw_json": "{}",
    }


def sync_core_lightning_wallet(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    backend: Mapping[str, Any],
    hooks: core_imports.ImportCoordinatorHooks,
    *,
    commit: bool = True,
    rpc_call: RpcCall | None = None,
) -> dict[str, Any]:
    """Refresh the Lightning node records for one ``coreln`` wallet."""
    if str(backend.get("kind") or "").lower() != CLN_BACKEND_KIND:
        raise AppError(
            f"Backend '{backend.get('name')}' has kind '{backend.get('kind')}', expected 'coreln'",
            code="validation",
            hint=(
                "Create a Core Lightning backend with"
                " `kassiber backends create <name> --kind coreln --url cln://local`."
            ),
        )
    started_at = now_iso()
    sync_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO lightning_node_syncs(
            id, workspace_id, profile_id, wallet_id, backend_name, started_at,
            status, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            sync_id,
            profile["workspace_id"],
            profile["id"],
            wallet["id"],
            backend["name"],
            started_at,
            "running",
            started_at,
        ),
    )
    snapshot = fetch_core_lightning_snapshot(backend, rpc_call=rpc_call)
    completed_at = now_iso()
    records = snapshot_records(snapshot, completed_at)
    inserted = 0
    updated = 0
    counts: dict[str, int] = {}
    import_records: list[dict[str, Any]] = []
    for record in records:
        counts[record["record_type"]] = counts.get(record["record_type"], 0) + 1
        was_inserted, was_updated = _upsert_lightning_record(
            conn,
            profile,
            wallet,
            backend,
            sync_id,
            snapshot.node_id,
            record,
            completed_at,
        )
        inserted += 1 if was_inserted else 0
        updated += 1 if was_updated else 0
        import_record = _record_to_import(record)
        if import_record is not None:
            import_records.append(import_record)
    import_outcome = core_imports.insert_wallet_records(
        conn,
        profile,
        wallet,
        import_records,
        CLN_IMPORT_SOURCE,
        hooks,
        commit=False,
    )
    status = "partial" if snapshot.errors else "synced"
    conn.execute(
        """
        UPDATE lightning_node_syncs
        SET node_id = ?, node_alias = ?, completed_at = ?, status = ?,
            fetched_counts_json = ?, error_json = ?
        WHERE id = ?
        """,
        (
            snapshot.node_id,
            snapshot.node_alias,
            completed_at,
            status,
            _json_dumps(counts),
            _json_dumps(snapshot.errors),
            sync_id,
        ),
    )
    invalidate_journals(conn, profile["id"])
    if commit:
        conn.commit()
    return {
        "wallet": wallet["label"],
        "backend": backend["name"],
        "backend_kind": CLN_BACKEND_KIND,
        "backend_url": redact_backend_url(backend["url"]),
        "node_id": snapshot.node_id,
        "node_alias": snapshot.node_alias,
        "status": status,
        "sync_id": sync_id,
        "started_at": started_at,
        "completed_at": completed_at,
        "records_fetched": len(records),
        "records_inserted": inserted,
        "records_updated": updated,
        "fetched_counts": counts,
        "method_errors": dict(snapshot.errors),
        "transactions": {
            "fetched": len(import_records),
            "imported": import_outcome["imported"],
            "skipped": import_outcome["skipped"],
            "updated": import_outcome.get("updated", 0),
        },
        "read_only": True,
    }


# --- Register at import time ----------------------------------------------

register_adapter("coreln", CoreLightningAdapter())


__all__ = [
    "CLN_ALLOWED_METHODS",
    "CLN_BACKEND_KIND",
    "CLN_IMPORT_SOURCE",
    "CLN_READONLY_RUNE_RESTRICTIONS",
    "CLN_WALLET_KIND",
    "CoreLightningAdapter",
    "CoreLightningSnapshot",
    "build_node_snapshot",
    "call_core_lightning",
    "fetch_core_lightning_snapshot",
    "snapshot_records",
    "sync_core_lightning_wallet",
]
