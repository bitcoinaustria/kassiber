"""LND read-only adapter for the shared Lightning scaffold.

This module implements the :class:`LightningAdapter` Protocol for the LND
REST API. It is strictly read-only — it never opens or closes channels,
never pays invoices, and never mutates node state.

Opsec policy (see [docs/reference/lightning-opsec.md](../../../docs/reference/lightning-opsec.md))
is applied at the adapter boundary:

- ``/v1/invoices`` — drop ``r_preimage`` (preimage), ``payment_request``
  (encoded bolt11 with `payment_secret` + route hints), ``payment_addr``
  (payment_secret), and the entire ``route_hints`` block. Keep decoded
  amount + description + ``r_hash`` (payment_hash) + timestamps.
- ``/v1/payments`` — drop ``payment_preimage``, ``payment_request``, and
  the per-HTLC ``route.hops`` lists (the full route is the
  deanonymization tool). Keep ``value_sat`` + ``fee_sat`` + destination
  pubkey + ``payment_hash``. For failed attempts, drop
  ``failure_source_pubkey`` — only the categorical ``failure_reason``
  survives.
- ``/v1/channels`` — for private channels (``private == true``), pass
  ``None`` for :attr:`NodeChannel.peer_pubkey` unless the operator opts
  in. Public-channel peer pubkeys are already in gossip.
- ``/v1/forwarding`` — aggregate per-day per-channel revenue for the
  :class:`NodeRoutingSnapshot` summary. Per-forward rows that reach
  :class:`NodeForward` keep only short channel ids + peer aliases (the
  shape has nowhere to put peer pubkeys, by construction).

The adapter registers itself with the shared registry on import:
``register_adapter("lnd", LndAdapter())``.
"""

from __future__ import annotations

import base64
import json
import ssl
from pathlib import Path
from typing import Any, Mapping
from urllib import error as urlerror
from urllib import parse as urlparse
from urllib import request as urlrequest

from ... import __version__
from ...backends import backend_timeout, backend_value
from ...db import APP_NAME
from ...egress_ledger import get_egress_ledger, http_request_bytes_out
from ...errors import AppError
from ...msat import msat_to_btc
from ...time_utils import UNKNOWN_OCCURRED_AT, now_iso, timestamp_to_iso
from .. import imports as core_imports
from ..repo import invalidate_journals
from .capabilities import LightningCapabilities
from .registry import register_adapter
from .types import (
    NodeChannel,
    NodeForward,
    NodeRoutingSnapshot,
    NodeSnapshot,
)

LND_DEFAULT_PAGE_SIZE = 100
LND_MAX_PAGE_SIZE = 1000
LND_DEFAULT_WINDOW_DAYS = 30
# Daily seconds shorthand for forwarding-window math.
_SECONDS_PER_DAY = 86_400
_LND_CHANNEL_ID_PREFIX = "lnd:"
_LIFECYCLE_AMOUNT_INCOMPLETE = -1


# ---------------------------------------------------------------------------
# Coercion helpers
# ---------------------------------------------------------------------------


def _int(value: Any, default: int = 0) -> int:
    if value in (None, ""):
        return default
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


def _msat(value: Any, *, sats: bool = False) -> int:
    """Coerce a value to msat (raw LND fields are sometimes sat strings)."""
    amount = _int(value)
    return amount * 1000 if sats else amount


def _timestamp(value: Any) -> str | None:
    """Convert an LND unix-seconds timestamp to ISO-8601 (or None)."""
    raw = _int(value)
    if raw <= 0:
        return None
    return timestamp_to_iso(raw)


def _is_truthy(value: Any) -> bool:
    """LND REST returns booleans as JSON True/False, but proto strings show
    up as 'true' / 'TRUE' on some paths. Be defensive."""
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        return value.strip().lower() in {"true", "1", "yes"}
    return bool(value)


# ---------------------------------------------------------------------------
# Backend resolution + REST client
# ---------------------------------------------------------------------------


def _macaroon_hex(backend: Mapping[str, Any]) -> str:
    value = backend_value(backend, "token")
    if not value:
        raise AppError(
            "LND backend is missing a read-only macaroon",
            code="config_error",
            hint=(
                "Store a scoped read-only macaroon with"
                " `backends create --kind lnd --token-stdin` or the desktop"
                " backend form."
            ),
            retryable=False,
        )
    cleaned = value.strip()
    if (
        all(char in "0123456789abcdefABCDEF" for char in cleaned)
        and len(cleaned) % 2 == 0
    ):
        return cleaned.lower()
    raise AppError(
        "LND macaroon must be stored as hex",
        code="validation",
        hint=(
            "Use"
            " `xxd -p -c 256 readonly.macaroon | kassiber backends create"
            " --kind lnd --token-stdin ...`."
        ),
        retryable=False,
    )


def _ssl_context(backend: Mapping[str, Any]) -> ssl.SSLContext | None:
    """Build the TLS context for an LND backend.

    Uses :func:`backend_value` so DB-resolved backends (where
    ``config_json`` is flattened to the top level by
    ``_backend_row_to_dict``) and synthetic dicts both work.
    """
    insecure = backend_value(backend, "insecure", "trust_ssl")
    if insecure is not None and _is_truthy(insecure):
        return ssl._create_unverified_context()
    cert = backend_value(backend, "certificate")
    if not cert:
        return None
    context = ssl.create_default_context()
    cert_path = Path(cert).expanduser()
    if cert_path.exists():
        context.load_verify_locations(cafile=str(cert_path))
    else:
        context.load_verify_locations(cadata=cert)
    return context


class LndRestClient:
    """Minimal LND REST client (GET / POST JSON, macaroon-authenticated).

    Read-only by construction: this module never calls write endpoints.
    Network errors and non-2xx responses are surfaced as
    :class:`AppError` with the shared ``backend_error`` code so the
    daemon can render a user-meaningful envelope.
    """

    def __init__(self, backend: Mapping[str, Any]):
        self.backend = backend
        url = (backend_value(backend, "url") or "").rstrip("/")
        if not url:
            raise AppError("LND backend URL is required", code="config_error")
        self.base_url = url
        self.timeout = backend_timeout(backend)
        self.macaroon = _macaroon_hex(backend)
        self.context = _ssl_context(backend)

    def request(
        self,
        method: str,
        path: str,
        *,
        params: Mapping[str, Any] | None = None,
        payload: Mapping[str, Any] | None = None,
    ) -> dict[str, Any]:
        query = ""
        if params:
            query = "?" + urlparse.urlencode(
                {k: v for k, v in params.items() if v not in (None, "")}
            )
        data = (
            json.dumps(payload or {}).encode("utf-8")
            if payload is not None
            else None
        )
        request = urlrequest.Request(
            f"{self.base_url}{path}{query}",
            data=data,
            method=method,
            headers={
                "Accept": "application/json",
                "Content-Type": "application/json",
                "Grpc-Metadata-macaroon": self.macaroon,
                "User-Agent": f"{APP_NAME}/{__version__}",
            },
        )
        get_egress_ledger().record_url(
            request.full_url,
            subsystem="sync",
            operation="http.request",
            method=method,
            bytes_out=http_request_bytes_out(request, method),
        )
        try:
            with urlrequest.urlopen(
                request, timeout=self.timeout, context=self.context
            ) as response:
                raw = response.read().decode("utf-8")
        except urlerror.HTTPError as exc:
            detail = exc.read().decode("utf-8", "replace") if exc.fp else ""
            hint = None
            if exc.code in (401, 403):
                hint = (
                    "Check that the macaroon is read-only and valid for this"
                    " node."
                )
            raise AppError(
                f"LND returned HTTP {exc.code}: {detail[:200]}",
                code="backend_error",
                hint=hint,
                retryable=500 <= exc.code < 600,
            ) from exc
        except urlerror.URLError as exc:
            raise AppError(
                f"Failed to reach LND backend: {exc.reason}",
                code="backend_error",
                hint=(
                    "Check the LND REST host, port, TLS certificate, and"
                    " network reachability."
                ),
                retryable=True,
            ) from exc
        if not raw:
            return {}
        try:
            payload_obj = json.loads(raw)
        except json.JSONDecodeError as exc:
            raise AppError(
                "LND returned non-JSON data",
                code="backend_error",
                details={"response_preview": raw[:120]},
                retryable=True,
            ) from exc
        if not isinstance(payload_obj, dict):
            raise AppError(
                "LND returned an unexpected response shape",
                code="backend_error",
                details={"response_type": type(payload_obj).__name__},
                retryable=True,
            )
        return payload_obj

    def get(
        self, path: str, *, params: Mapping[str, Any] | None = None
    ) -> dict[str, Any]:
        return self.request("GET", path, params=params)

    def post(
        self, path: str, payload: Mapping[str, Any] | None = None
    ) -> dict[str, Any]:
        return self.request("POST", path, payload=payload or {})


# ---------------------------------------------------------------------------
# REST-row → scaffold-shape mapping
# ---------------------------------------------------------------------------


def _channel_state(row: Mapping[str, Any], *, closed: bool) -> str:
    if closed:
        close_type = str(row.get("close_type") or "").upper()
        if "BREACH" in close_type:
            return "force_closed"
        if "FORCE" in close_type:
            return "force_closed"
        return "closed"
    if _is_truthy(row.get("active")):
        return "active"
    return "inactive"


def _channel_close_kind(
    row: Mapping[str, Any],
) -> str | None:
    close_type = str(row.get("close_type") or "").upper()
    if "BREACH" in close_type:
        return "breach"
    if "FORCE" in close_type or "ABANDONED" in close_type:
        return "force"
    if close_type:
        return "cooperative"
    return None


def _map_channel(
    row: Mapping[str, Any],
    *,
    closed: bool,
    fee_lookup: Mapping[str, dict[str, int]] | None = None,
    forwards_per_channel: Mapping[str, dict[str, int]] | None = None,
    peer_alias_lookup: Mapping[str, str] | None = None,
) -> NodeChannel:
    chan_id = str(row.get("chan_id") or "") or None
    channel_point = (
        row.get("channel_point")
        or row.get("funding_txid")
        or row.get("closing_tx_hash")
    )
    is_private = _is_truthy(row.get("private"))
    remote_pubkey = row.get("remote_pubkey") or None
    # Opsec: private channels default to peer_pubkey=None even if the
    # gossip exposes the value via LND. Only public channels surface it.
    peer_pubkey = None if is_private else remote_pubkey
    fee_row = (fee_lookup or {}).get(str(chan_id or ""), {})
    forward_row = (forwards_per_channel or {}).get(str(chan_id or ""), {})
    channel_id = chan_id or str(channel_point or "")
    # Opsec: for private channels, the alias fallback must never reach
    # back to `remote_pubkey` — that would leak the pubkey under a
    # different field and bypass the `peer_pubkey=None` guard above. We
    # use the same neutral placeholder the desktop shows for null
    # `peer_pubkey` (see NodeConnectionDetail.tsx). When LND DID give us
    # an alias for a private channel we keep it as-is: that's the peer's
    # chosen identity, and the opsec rule is to not leak the pubkey via
    # FALLBACK, not to second-guess what LND surfaces.
    raw_alias = row.get("peer_alias")
    resolved_alias = (peer_alias_lookup or {}).get(str(remote_pubkey or ""))
    if is_private:
        peer_alias = str(raw_alias or resolved_alias or "private peer")
    else:
        peer_alias = str(raw_alias or resolved_alias or remote_pubkey or "unknown")
    return NodeChannel(
        id=channel_id,
        peer_alias=peer_alias,
        peer_pubkey=peer_pubkey,
        capacity_sat=_int(row.get("capacity")),
        local_balance_sat=_int(row.get("local_balance")),
        remote_balance_sat=_int(row.get("remote_balance")),
        state=_channel_state(row, closed=closed),
        is_private=is_private,
        is_initiator=_is_truthy(row.get("initiator")),
        short_channel_id=chan_id,
        funding_outpoint=channel_point or None,
        base_fee_msat=_int(fee_row.get("base_fee_msat")) or None,
        fee_rate_ppm=_int(fee_row.get("fee_rate_ppm")) or None,
        opened_at=None,
        closed_at=(
            _timestamp(row.get("close_time") or row.get("close_timestamp"))
            if closed
            else None
        ),
        close_kind=_channel_close_kind(row) if closed else None,
        forward_count=forward_row.get("count"),
        earned_routing_sat=forward_row.get("earned_sat"),
        htlc_count=len(row.get("pending_htlcs") or []) or None,
        last_activity_at=None,
    )


def _scoped_forwards(
    payload: Mapping[str, Any],
    *,
    window_start_ts: int,
) -> list[dict[str, Any]]:
    rows = payload.get("forwarding_events") or []
    scoped: list[dict[str, Any]] = []
    for row in rows:
        if not isinstance(row, dict):
            continue
        ts = _int(row.get("timestamp"))
        if window_start_ts and ts and ts < window_start_ts:
            continue
        scoped.append(row)
    return scoped


def _fee_lookup(payload: Mapping[str, Any]) -> dict[str, dict[str, int]]:
    out: dict[str, dict[str, int]] = {}
    for entry in payload.get("channel_fees") or []:
        if not isinstance(entry, dict):
            continue
        chan_id = str(entry.get("chan_id") or entry.get("channel_id") or "")
        if not chan_id:
            continue
        out[chan_id] = {
            "base_fee_msat": _int(entry.get("base_fee_msat")),
            # LND returns fee_per_mil as ppm.
            "fee_rate_ppm": _int(entry.get("fee_per_mil")),
        }
    return out


def _forwards_per_channel(
    forwards: list[dict[str, Any]],
) -> dict[str, dict[str, int]]:
    out: dict[str, dict[str, int]] = {}
    for row in forwards:
        chan_out = str(row.get("chan_id_out") or "")
        if not chan_out:
            continue
        slot = out.setdefault(chan_out, {"count": 0, "earned_sat": 0})
        slot["count"] += 1
        # Aggregate the routing fee earned on the outbound side; LND
        # exposes both fee_msat and fee (sats) — prefer msat when present.
        fee_msat = _int(row.get("fee_msat"))
        if not fee_msat:
            fee_msat = _msat(row.get("fee"), sats=True)
        slot["earned_sat"] += fee_msat // 1000
    return out


def _peer_alias_lookup(
    client: LndRestClient,
    rows: list[dict[str, Any]],
) -> dict[str, str]:
    """Resolve peer aliases for private LND channels without exposing pubkeys.

    Some LND builds omit ``peer_alias`` from ``listchannels`` for private
    channels even when the peer's node announcement is known through the graph.
    The lookup map is used only while building ``peer_alias``; private-channel
    ``peer_pubkey`` remains ``None`` in the emitted scaffold shape.
    """

    aliases: dict[str, str] = {}
    pubkeys = {
        str(row.get("remote_pubkey") or "")
        for row in rows
        if _is_truthy(row.get("private"))
        and row.get("remote_pubkey")
        and not row.get("peer_alias")
    }
    for pubkey in sorted(pubkeys):
        try:
            payload = client.get(
                f"/v1/graph/node/{urlparse.quote(pubkey, safe='')}"
            )
        except AppError:
            continue
        node = payload.get("node")
        if not isinstance(node, Mapping):
            continue
        alias = str(node.get("alias") or "").strip()
        if alias:
            aliases[pubkey] = alias
    return aliases


def _map_forward(row: Mapping[str, Any]) -> NodeForward:
    occurred_at = _timestamp(row.get("timestamp")) or "1970-01-01T00:00:00Z"
    in_chan = str(row.get("chan_id_in") or "") or None
    out_chan = str(row.get("chan_id_out") or "") or None
    amt_in_msat = _int(row.get("amt_in_msat"))
    if not amt_in_msat:
        amt_in_msat = _msat(row.get("amt_in"), sats=True)
    amt_out_msat = _int(row.get("amt_out_msat"))
    if not amt_out_msat:
        amt_out_msat = _msat(row.get("amt_out"), sats=True)
    fee_msat = _int(row.get("fee_msat"))
    if not fee_msat:
        fee_msat = _msat(row.get("fee"), sats=True)
    return NodeForward(
        # Opsec: short channel ids + peer aliases only; never peer
        # pubkeys. Aliases come from LND when available.
        id=f"{row.get('timestamp_ns') or row.get('timestamp') or ''}:{in_chan or '?'}:{out_chan or '?'}",
        occurred_at=occurred_at,
        in_peer_alias=str(row.get("peer_alias_in") or in_chan or "unknown"),
        out_peer_alias=str(row.get("peer_alias_out") or out_chan or "unknown"),
        amount_in_msat=amt_in_msat,
        amount_out_msat=amt_out_msat,
        fee_msat=fee_msat,
        status="settled",
        in_short_channel_id=in_chan,
        out_short_channel_id=out_chan,
    )


def _summarize_routing(
    forwards: list[dict[str, Any]],
    payments: list[dict[str, Any]],
    *,
    window_days: int,
    onchain_cost_sat: int,
) -> NodeRoutingSnapshot:
    routing_revenue_msat = 0
    for row in forwards:
        fee_msat = _int(row.get("fee_msat"))
        if not fee_msat:
            fee_msat = _msat(row.get("fee"), sats=True)
        routing_revenue_msat += fee_msat
    payment_cost_sat = 0
    payment_count = 0
    rebalance_count = 0
    for row in payments:
        if str(row.get("status") or "").upper() not in {
            "SUCCEEDED",
            "SUCCESS",
            "COMPLETE",
        }:
            continue
        payment_count += 1
        fee_msat = _int(row.get("fee_msat"))
        if not fee_msat:
            fee_msat = _msat(row.get("fee_sat"), sats=True)
        payment_cost_sat += fee_msat // 1000
    routing_revenue_sat = routing_revenue_msat // 1000
    # Rebalance attribution requires self-payment detection; leave the
    # rebalance bucket at zero by default and surface a `rebalance_count`
    # of zero so the report can present it as "not yet classified".
    rebalance_cost_sat = 0
    net_profit_sat = (
        routing_revenue_sat
        - payment_cost_sat
        - rebalance_cost_sat
        - onchain_cost_sat
    )
    return NodeRoutingSnapshot(
        window_label=f"Last {window_days} days",
        routing_revenue_sat=routing_revenue_sat,
        payment_cost_sat=payment_cost_sat,
        rebalance_cost_sat=rebalance_cost_sat,
        onchain_cost_sat=onchain_cost_sat,
        net_profit_sat=net_profit_sat,
        forward_count=len(forwards),
        payment_count=payment_count,
        rebalance_count=rebalance_count,
    )


# ---------------------------------------------------------------------------
# Tier-1 sanitization (called before any LND payload reaches scaffold types)
# ---------------------------------------------------------------------------

_INVOICE_DROP_FIELDS = (
    "r_preimage",
    "payment_request",
    "payment_addr",
    "route_hints",
)
_PAYMENT_DROP_FIELDS = (
    "payment_preimage",
    "payment_request",
)


def _sanitize_invoice(row: Mapping[str, Any]) -> dict[str, Any]:
    """Drop opsec-banned fields from an invoice row before persistence."""
    cleaned: dict[str, Any] = {
        key: value
        for key, value in row.items()
        if key not in _INVOICE_DROP_FIELDS
    }
    return cleaned


def _sanitize_payment(row: Mapping[str, Any]) -> dict[str, Any]:
    """Drop preimage / encoded bolt11 / hop list / failure source pubkey."""
    cleaned: dict[str, Any] = {
        key: value
        for key, value in row.items()
        if key not in _PAYMENT_DROP_FIELDS
    }
    htlcs = []
    for htlc in row.get("htlcs") or []:
        if not isinstance(htlc, Mapping):
            continue
        htlc_clean = dict(htlc)
        # The full route is the deanonymization tool. Keep the categorical
        # failure code; drop the per-hop list and any source-pubkey.
        route = htlc_clean.get("route")
        if isinstance(route, Mapping):
            route_clean = dict(route)
            route_clean.pop("hops", None)
            htlc_clean["route"] = route_clean
        htlc_clean.pop("failure_source_pubkey", None)
        htlcs.append(htlc_clean)
    if htlcs:
        cleaned["htlcs"] = htlcs
    cleaned.pop("failure_source_pubkey", None)
    return cleaned


def _invoice_counts(invoices: list[dict[str, Any]]) -> tuple[int, int, int]:
    paid = 0
    expired = 0
    for invoice in invoices:
        state = str(invoice.get("state") or "").upper()
        if _is_truthy(invoice.get("settled")) or state == "SETTLED":
            paid += 1
        elif state == "EXPIRED":
            expired += 1
    return len(invoices), paid, expired


def _payment_counts(payments: list[dict[str, Any]]) -> tuple[int, int, int]:
    completed = 0
    failed = 0
    for payment in payments:
        status = str(payment.get("status") or "").upper()
        if status in {"SUCCEEDED", "SUCCESS", "COMPLETE"}:
            completed += 1
        elif status in {"FAILED", "FAILURE"}:
            failed += 1
    return len(payments), completed, failed


# ---------------------------------------------------------------------------
# Adapter
# ---------------------------------------------------------------------------


def _normalize_network(value: Any) -> str:
    raw = str(value or "").strip().lower()
    if not raw:
        return "mainnet"
    if raw in {"main", "mainnet"}:
        return "mainnet"
    if raw in {"test", "testnet"}:
        return "testnet"
    if raw in {"sig", "signet"}:
        return "signet"
    if raw in {"reg", "regtest"}:
        return "regtest"
    return raw


def _chains_to_network(info: Mapping[str, Any]) -> str:
    chains = info.get("chains") or []
    for entry in chains:
        if isinstance(entry, Mapping) and entry.get("network"):
            return _normalize_network(entry.get("network"))
    return _normalize_network(info.get("testnet"))


class LndAdapter:
    """Read-only LND adapter.

    Construction is cheap — the REST client is built lazily from the
    backend row each call so test doubles can monkey-patch
    :class:`LndRestClient`.
    """

    kind = "lnd"
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
        window_days: int = LND_DEFAULT_WINDOW_DAYS,
    ) -> NodeSnapshot:
        if not backend:
            raise AppError(
                "LND adapter requires a backend row for the connection.",
                code="config_error",
                hint=(
                    "Attach a backend (kind=lnd) to the wallet before"
                    " requesting a node snapshot."
                ),
                retryable=False,
            )
        if str(backend.get("kind") or "").lower() != "lnd":
            raise AppError(
                f"Backend '{backend.get('name')}' has kind"
                f" '{backend.get('kind')}', expected 'lnd'",
                code="validation",
                retryable=False,
            )
        window_days = max(1, int(window_days or LND_DEFAULT_WINDOW_DAYS))
        client = LndRestClient(backend)

        info = client.get("/v1/getinfo")
        onchain = client.get("/v1/balance/blockchain")
        channel_balance = client.get("/v1/balance/channels")
        fees_payload = client.get("/v1/fees")
        fee_lookup = _fee_lookup(fees_payload)

        # Open channels.
        open_channels_payload = client.get("/v1/channels")
        open_rows = [
            row
            for row in (open_channels_payload.get("channels") or [])
            if isinstance(row, dict)
        ]

        # Closed channels.
        closed_channels_payload = client.get("/v1/channels/closed")
        closed_rows = [
            row
            for row in (closed_channels_payload.get("channels") or [])
            if isinstance(row, dict)
        ]

        # Forwarding events (bounded by window_days). LND's
        # /v1/switch supports start_time / end_time so we don't pull the
        # whole history just to summarize the recent window.
        now_ts = _int(info.get("best_header_timestamp")) or 0
        start_ts = (
            max(0, now_ts - window_days * _SECONDS_PER_DAY) if now_ts else 0
        )
        forwarding_payload = client.post(
            "/v1/switch",
            {
                "start_time": str(start_ts),
                "end_time": str(now_ts) if now_ts else "",
                "num_max_events": LND_MAX_PAGE_SIZE,
            },
        )
        forwards = _scoped_forwards(
            forwarding_payload, window_start_ts=start_ts
        )

        # Payments (window-scoped — we only need successful ones for the
        # routing summary, but the adapter sanitizes every row that comes
        # through so future itemization stays opsec-clean).
        payments_payload = client.get(
            "/v1/payments",
            params={
                "include_incomplete": "true",
                "max_payments": LND_MAX_PAGE_SIZE,
            },
        )
        payments_raw = [
            row
            for row in (payments_payload.get("payments") or [])
            if isinstance(row, dict)
        ]
        payments_clean = [_sanitize_payment(row) for row in payments_raw]
        if start_ts:
            payments_clean = [
                row
                for row in payments_clean
                if _int(row.get("creation_time_ns")) // 1_000_000_000
                >= start_ts
                or _int(row.get("creation_date")) >= start_ts
            ]

        # Invoices: sanitize before counting so the discard policy is the
        # single boundary for every payload that contributes to the snapshot.
        invoices_payload = client.get(
            "/v1/invoices",
            params={"num_max_invoices": LND_MAX_PAGE_SIZE},
        )
        invoices_clean = [
            _sanitize_invoice(row)
            for row in (invoices_payload.get("invoices") or [])
            if isinstance(row, dict)
        ]
        invoice_count, paid_invoice_count, expired_invoice_count = _invoice_counts(invoices_clean)
        payment_count, completed_payment_count, failed_payment_count = _payment_counts(payments_clean)

        # Per-channel forward aggregates (used for break-even view).
        forwards_per_channel = _forwards_per_channel(forwards)
        peer_alias_lookup = _peer_alias_lookup(client, open_rows + closed_rows)

        channels = tuple(
            _map_channel(
                row,
                closed=False,
                fee_lookup=fee_lookup,
                forwards_per_channel=forwards_per_channel,
                peer_alias_lookup=peer_alias_lookup,
            )
            for row in open_rows
        )
        closed_channels = tuple(
            _map_channel(
                row,
                closed=True,
                peer_alias_lookup=peer_alias_lookup,
            )
            for row in closed_rows
        )

        onchain_cost_sat = sum(
            row.get("commit_fee_sat", 0) for row in [{}]
        )  # placeholder — onchain cost attribution is future work.
        routing = _summarize_routing(
            forwards,
            payments_clean,
            window_days=window_days,
            onchain_cost_sat=onchain_cost_sat,
        )

        local_balance_sat = 0
        remote_balance_sat = 0
        if isinstance(channel_balance.get("local_balance"), dict):
            local_balance_sat = _int(
                channel_balance["local_balance"].get("sat")
            )
        else:
            local_balance_sat = _int(channel_balance.get("balance"))
        if isinstance(channel_balance.get("remote_balance"), dict):
            remote_balance_sat = _int(
                channel_balance["remote_balance"].get("sat")
            )
        else:
            remote_balance_sat = _int(channel_balance.get("remote_balance"))

        return NodeSnapshot(
            alias=str(info.get("alias") or ""),
            pubkey=str(info.get("identity_pubkey") or ""),
            network=_chains_to_network(info),
            implementation_version=str(info.get("version") or "") or None,
            peer_count=_int(info.get("num_peers")),
            block_height=_int(info.get("block_height")) or None,
            onchain_balance_sat=_int(onchain.get("confirmed_balance")),
            total_local_balance_sat=local_balance_sat,
            total_remote_balance_sat=remote_balance_sat,
            total_capacity_sat=sum(channel.capacity_sat for channel in channels),
            channels=channels,
            closed_channels=closed_channels,
            invoice_count=invoice_count,
            paid_invoice_count=paid_invoice_count,
            expired_invoice_count=expired_invoice_count,
            payment_count=payment_count,
            completed_payment_count=completed_payment_count,
            failed_payment_count=failed_payment_count,
            routing=routing,
            forwards=tuple(_map_forward(row) for row in forwards),
        )


LND_IMPORT_SOURCE = "lnd"

_HEX_DIGITS = frozenset("0123456789abcdefABCDEF")


def _normalize_lnd_hash(value: Any) -> str | None:
    """Return a lowercase-hex payment hash from an LND ``r_hash``/``payment_hash``.

    LND's gRPC ``payment_hash`` is hex, but the REST gateway base64-encodes the
    32-byte ``r_hash`` on invoices. Normalizing both to hex is what lets an
    own-node CLN<->LND payment pair by hash: the CLN side stores hex, so the LND
    legs must match. Returns ``None`` when the value is neither 32-byte hex nor
    32-byte base64 (so unmatched rows simply stay unpaired rather than mispair).
    """
    if not value:
        return None
    text = str(value).strip()
    if len(text) == 64 and all(char in _HEX_DIGITS for char in text):
        return text.lower()
    for decoder in (base64.urlsafe_b64decode, base64.b64decode):
        try:
            raw = decoder(text + "=" * (-len(text) % 4))
        except Exception:  # noqa: BLE001 - defensive base64 decode
            continue
        if len(raw) == 32:
            return raw.hex()
    return None


def _lnd_invoice_import(invoice: Mapping[str, Any]) -> dict[str, Any] | None:
    """Promote a settled LND invoice to an inbound wallet transaction.

    Mirrors the Core Lightning invoice-income promotion so LND income is booked
    and can pair (by payment hash) with an own-node outbound leg.
    """
    state = str(invoice.get("state") or "").upper()
    if not (_is_truthy(invoice.get("settled")) or state == "SETTLED"):
        return None
    amount_msat = _msat(invoice.get("amt_paid_msat") or invoice.get("value_msat"))
    if amount_msat <= 0:
        amount_msat = _msat(invoice.get("amt_paid_sat") or invoice.get("value"), sats=True)
    if amount_msat <= 0:
        return None
    payment_hash = _normalize_lnd_hash(invoice.get("r_hash"))
    external_id = payment_hash or str(
        invoice.get("settle_index")
        or invoice.get("add_index")
        or invoice.get("creation_date")
        or ""
    )
    if not external_id:
        return None
    occurred_at = (
        _timestamp(invoice.get("settle_date") or invoice.get("creation_date"))
        or UNKNOWN_OCCURRED_AT
    )
    return {
        "id": f"lnd:invoice:{external_id}",
        "occurred_at": occurred_at,
        "confirmed_at": occurred_at,
        "direction": "inbound",
        "asset": "BTC",
        "amount": msat_to_btc(amount_msat),
        "fee": 0,
        "kind": "lnd_invoice",
        "description": str(invoice.get("memo") or "LND invoice"),
        "counterparty": None,
        "payment_hash": payment_hash,
        "payment_hash_source": "lnd" if payment_hash else None,
        "raw_json": "{}",
    }


def _lnd_payment_import(payment: Mapping[str, Any]) -> dict[str, Any] | None:
    """Promote a succeeded LND payment to an outbound wallet transaction.

    ``value_msat`` is the principal and ``fee_msat`` the routing fee — the same
    split Core Lightning uses, so an own-node payment nets to a transfer whose
    only taxable component is the routing fee.
    """
    status = str(payment.get("status") or "").upper()
    if status not in {"SUCCEEDED", "COMPLETE", "COMPLETED"}:
        return None
    amount_msat = _msat(payment.get("value_msat"))
    if amount_msat <= 0:
        amount_msat = _msat(payment.get("value"), sats=True)
    if amount_msat <= 0:
        return None
    fee_msat = _msat(payment.get("fee_msat"))
    if fee_msat <= 0:
        fee_msat = _msat(payment.get("fee"), sats=True)
    payment_hash = _normalize_lnd_hash(payment.get("payment_hash"))
    external_id = payment_hash or str(
        payment.get("payment_index") or payment.get("creation_date") or ""
    )
    if not external_id:
        return None
    created = payment.get("creation_date")
    if not created and payment.get("creation_time_ns"):
        created = _int(payment.get("creation_time_ns")) // 1_000_000_000
    occurred_at = _timestamp(created) or UNKNOWN_OCCURRED_AT
    return {
        "id": f"lnd:pay:{external_id}",
        "occurred_at": occurred_at,
        "confirmed_at": occurred_at,
        "direction": "outbound",
        "asset": "BTC",
        "amount": msat_to_btc(amount_msat),
        "fee": msat_to_btc(fee_msat),
        "kind": "lnd_pay",
        "description": "LND payment",
        "counterparty": None,
        "payment_hash": payment_hash,
        "payment_hash_source": "lnd" if payment_hash else None,
        "raw_json": "{}",
    }


def lnd_import_records(
    invoices: list[Mapping[str, Any]],
    payments: list[Mapping[str, Any]],
    forwards: list[Mapping[str, Any]] | None = None,
) -> list[dict[str, Any]]:
    """Build LND ledger rows, including daily fee-only routing income."""
    records: list[dict[str, Any]] = []
    for invoice in invoices:
        record = _lnd_invoice_import(invoice)
        if record is not None:
            records.append(record)
    for payment in payments:
        record = _lnd_payment_import(payment)
        if record is not None:
            records.append(record)
    records.extend(_lnd_routing_income_imports(forwards or []))
    return records


def _stamp_lightning_import_network(
    records: list[dict[str, Any]], network: str
) -> list[dict[str, Any]]:
    """Attach adapter-observed network scope to curated ledger records."""

    if not str(network or "").strip():
        return records
    stamped = []
    for record in records:
        item = dict(record)
        raw = item.get("raw_json")
        try:
            payload = json.loads(raw) if isinstance(raw, str) else dict(raw or {})
        except (TypeError, ValueError, json.JSONDecodeError):
            payload = {}
        payload.update({"chain": "lightning", "network": network})
        item["raw_json"] = json.dumps(payload, sort_keys=True)
        stamped.append(item)
    return stamped


def _lnd_routing_income_imports(
    forwards: list[Mapping[str, Any]],
) -> list[dict[str, Any]]:
    """Aggregate settled forwarding revenue to one fee-only row per UTC day.

    Forwarded principal is never income and is intentionally discarded at this
    boundary.  Daily aggregation prevents the accounting ledger from becoming
    a reconstructable per-payment routing graph.
    """
    fee_by_day: dict[str, int] = {}
    for row in forwards:
        raw_ts = _int(row.get("timestamp"))
        if raw_ts <= 0:
            raw_ts = _int(row.get("timestamp_ns")) // 1_000_000_000
        occurred_at = _timestamp(raw_ts)
        if not occurred_at:
            continue
        fee_msat = _int(row.get("fee_msat"))
        if fee_msat <= 0:
            fee_msat = _msat(row.get("fee"), sats=True)
        if fee_msat <= 0:
            continue
        day = occurred_at[:10]
        fee_by_day[day] = fee_by_day.get(day, 0) + fee_msat
    return [
        {
            "id": f"lnd:routing:{day}",
            # Preserve daily aggregation privacy while ensuring fee inventory
            # is never made available before the forwards that earned it.
            "occurred_at": f"{day}T23:59:59Z",
            "confirmed_at": f"{day}T23:59:59Z",
            "direction": "inbound",
            "asset": "BTC",
            "amount": msat_to_btc(fee_msat),
            "fee": 0,
            "kind": "routing_income",
            "description": f"Lightning routing fees ({day})",
            "counterparty": None,
            "payment_hash": None,
            "payment_hash_source": None,
            "raw_json": "{}",
        }
        for day, fee_msat in sorted(fee_by_day.items())
    ]


def _fetch_lnd_forwarding_history(client: "LndRestClient") -> list[dict[str, Any]]:
    """Read the full forwarding history with offset pagination.

    The raw rows live only in memory and are immediately reduced to daily fee
    totals by :func:`_lnd_routing_income_imports`.
    """
    offset = 0
    rows: list[dict[str, Any]] = []
    seen: set[tuple[str, str, str]] = set()
    while True:
        payload = client.post(
            "/v1/switch",
            {
                "start_time": "0",
                "index_offset": str(offset),
                "num_max_events": LND_MAX_PAGE_SIZE,
            },
        )
        page = [
            row
            for row in (payload.get("forwarding_events") or [])
            if isinstance(row, dict)
        ]
        for row in page:
            key = (
                str(row.get("timestamp_ns") or row.get("timestamp") or ""),
                str(row.get("chan_id_in") or ""),
                str(row.get("chan_id_out") or ""),
            )
            if key in seen:
                continue
            seen.add(key)
            rows.append(row)
        next_offset = _int(
            payload.get("last_offset_index") or payload.get("last_index_offset")
        )
        if not page or next_offset <= offset or len(page) < LND_MAX_PAGE_SIZE:
            break
        offset = next_offset
    return rows


def _fetch_lnd_payments(client: "LndRestClient") -> list[dict[str, Any]]:
    """Read the complete payment history in stable payment-index order."""
    return _fetch_lnd_indexed_history(
        client,
        path="/v1/payments",
        rows_key="payments",
        page_size_key="max_payments",
        fixed_params={"include_incomplete": "false"},
        identity_fields=("payment_index", "payment_hash"),
    )


def _fetch_lnd_invoices(client: "LndRestClient") -> list[dict[str, Any]]:
    """Read the complete invoice history in stable add-index order."""
    return _fetch_lnd_indexed_history(
        client,
        path="/v1/invoices",
        rows_key="invoices",
        page_size_key="num_max_invoices",
        fixed_params={},
        identity_fields=("add_index", "r_hash"),
    )


def _fetch_lnd_indexed_history(
    client: "LndRestClient",
    *,
    path: str,
    rows_key: str,
    page_size_key: str,
    fixed_params: Mapping[str, Any],
    identity_fields: tuple[str, ...],
) -> list[dict[str, Any]]:
    """Page an LND list endpoint without a long-history truncation ceiling."""
    offset = 0
    rows: list[dict[str, Any]] = []
    seen: set[tuple[str, ...]] = set()
    while True:
        payload = client.get(
            path,
            params={
                **fixed_params,
                "index_offset": str(offset),
                page_size_key: LND_MAX_PAGE_SIZE,
                "reversed": "false",
            },
        )
        page = [row for row in (payload.get(rows_key) or []) if isinstance(row, dict)]
        for row in page:
            identity = tuple(str(row.get(field) or "") for field in identity_fields)
            if identity in seen:
                continue
            seen.add(identity)
            rows.append(row)
        next_offset = _int(
            payload.get("last_index_offset") or payload.get("last_offset_index")
        )
        if not page or next_offset <= offset:
            break
        offset = next_offset
    return rows


def _lnd_funding_txid(row: Mapping[str, Any]) -> str | None:
    channel_point = str(row.get("channel_point") or "")
    if not channel_point:
        return None
    return channel_point.split(":", 1)[0] or None


def _lnd_local_initiator(row: Mapping[str, Any]) -> bool | None:
    if "initiator" in row:
        return _is_truthy(row.get("initiator"))
    value = str(row.get("open_initiator") or "").strip().upper()
    if value in {"INITIATOR_LOCAL", "LOCAL"}:
        return True
    if value in {"INITIATOR_REMOTE", "REMOTE"}:
        return False
    return None


def _lnd_channel_id(row: Mapping[str, Any]) -> str:
    # The funding outpoint is stable across the open/closed REST resources and
    # cannot collide with a CLN account id in a multi-node profile.
    channel_point = str(row.get("channel_point") or "").strip()
    fallback = str(row.get("chan_id") or row.get("channel_id") or "").strip()
    return f"{_LND_CHANNEL_ID_PREFIX}{channel_point or fallback}"


def _lnd_channel_record(
    tag: str,
    txid: str,
    row: Mapping[str, Any],
    amount_msat: int,
    network: str | None = None,
) -> dict[str, Any]:
    """A ``channel`` metadata record carrying one channel-lifecycle txid.

    Mirrors the Core Lightning channel record shape so the tax engine's
    channel-lifecycle netting is adapter-agnostic. Not a wallet transaction.
    """
    observed_scope = (
        {"chain": "bitcoin", "network": str(network).strip()}
        if str(network or "").strip()
        else {}
    )
    return {
        "record_type": "channel",
        "external_id": f"channel:{tag}:{_lnd_channel_id(row)}:{txid}",
        "tag": tag,
        "txid": txid,
        "outpoint": str(row.get("channel_point") or "") or None,
        "channel_id": _lnd_channel_id(row),
        "amount_msat": int(amount_msat),
        "status": "complete" if amount_msat > 0 else "incomplete",
        # Curated physical scope only — never persist the raw REST channel
        # payload. Lifecycle matching must still work when backend/wallet
        # configuration omits a network, especially on regtest.
        "raw_json": json.dumps(observed_scope, sort_keys=True),
    }


def lnd_channel_records(
    open_channels: list[Mapping[str, Any]],
    closed_channels: list[Mapping[str, Any]],
    *,
    network: str | None = None,
) -> list[dict[str, Any]]:
    """Build amount-bearing, channel-linked LND lifecycle evidence.

    ``capacity`` is not proof of the user's initial owned balance: a local
    initiator may push value to the peer at open or use leased/dual funding.
    Only an explicit adapter-provided local contribution is safe. Stock LND REST
    does not currently expose that historical value, so those opens stay typed
    incomplete and the lifecycle classifier quarantines rather than suppressing
    the L1 row. Remote-funded openings are not local on-chain outflows.
    """
    records_by_id: dict[str, dict[str, Any]] = {}
    for row in [*open_channels, *closed_channels]:
        funding = _lnd_funding_txid(row)
        local_initiator = _lnd_local_initiator(row)
        if funding and local_initiator is not False:
            local_contribution_sat = _int(
                row.get("local_funding_amount_sat")
                or row.get("local_contribution_sat")
            )
            amount_msat = (
                local_contribution_sat * 1000
                if local_initiator is True and local_contribution_sat > 0
                else _LIFECYCLE_AMOUNT_INCOMPLETE
            )
            record = _lnd_channel_record(
                "channel_open", funding, row, amount_msat, network
            )
            records_by_id[record["external_id"]] = record

    for row in closed_channels:
        closing = str(row.get("closing_tx_hash") or "").strip()
        if not closing:
            continue
        # Force closes can return an immediate settled part plus timelocked
        # outputs swept later.  Both remain our node balance at close.
        close_balance_sat = _int(row.get("settled_balance")) + _int(
            row.get("time_locked_balance")
        )
        amount_msat = (
            close_balance_sat * 1000
            if close_balance_sat > 0
            else _LIFECYCLE_AMOUNT_INCOMPLETE
        )
        record = _lnd_channel_record(
            "channel_close", closing, row, amount_msat, network
        )
        records_by_id[record["external_id"]] = record
    return [records_by_id[key] for key in sorted(records_by_id)]


def _persist_lnd_channel_records(
    conn: Any,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    backend: Mapping[str, Any],
    records: list[Mapping[str, Any]],
    timestamp: str,
) -> None:
    for record in records:
        external_id = str(record["external_id"])
        record_id = f"{wallet['id']}:{backend['name']}:channel:{external_id}"
        conn.execute(
            """
            INSERT INTO lightning_node_records(
                id, workspace_id, profile_id, wallet_id, backend_name, node_id,
                record_type, external_id, occurred_at, account, peer_id, channel_id,
                direction, amount_msat, fee_msat, tag, status, currency, payment_hash,
                txid, outpoint, sync_id, raw_json, first_seen_at, updated_at
            ) VALUES(?, ?, ?, ?, ?, NULL, 'channel', ?, ?, NULL, NULL, ?,
                     '', ?, 0, ?, ?, 'bc', NULL, ?, ?, NULL, ?, ?, ?)
            ON CONFLICT(profile_id, wallet_id, backend_name, record_type, external_id)
            DO UPDATE SET txid = excluded.txid, outpoint = excluded.outpoint,
                          channel_id = excluded.channel_id,
                          amount_msat = excluded.amount_msat,
                          tag = excluded.tag, status = excluded.status,
                          raw_json = excluded.raw_json,
                          updated_at = excluded.updated_at
            """,
            (
                record_id,
                profile["workspace_id"],
                profile["id"],
                wallet["id"],
                backend["name"],
                external_id,
                UNKNOWN_OCCURRED_AT,
                record.get("channel_id"),
                int(record.get("amount_msat") or 0),
                str(record["tag"]),
                str(record.get("status") or ""),
                str(record["txid"]),
                record.get("outpoint"),
                str(record.get("raw_json") or "{}"),
                timestamp,
                timestamp,
            ),
        )


def sync_lnd_wallet(
    conn: Any,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    backend: Mapping[str, Any],
    hooks: "core_imports.ImportCoordinatorHooks",
    *,
    commit: bool = True,
    client: "LndRestClient | None" = None,
) -> dict[str, Any]:
    """Import settled invoices and succeeded payments for one ``lnd`` wallet.

    Unlike Core Lightning, the LND node dashboard is served live from
    :meth:`LndAdapter.fetch_node_snapshot`, so this sync only promotes ledger
    transactions (income + payments); it does not persist snapshot aggregates.
    Payments and invoices carry the payment hash so own-node CLN<->LND transfers
    net via :func:`kassiber.transfers.detect_intra_transfers`.
    """
    if str(backend.get("kind") or "").lower() != "lnd":
        raise AppError(
            f"Backend '{backend.get('name')}' has kind '{backend.get('kind')}', expected 'lnd'",
            code="validation",
            hint=(
                "Create an LND backend with"
                " `kassiber backends create <name> --kind lnd --url https://... --token-stdin`."
            ),
        )
    if client is None:
        client = LndRestClient(backend)

    node_info = client.get("/v1/getinfo")
    node_network = _chains_to_network(node_info)

    payments = [
        _sanitize_payment(row)
        for row in _fetch_lnd_payments(client)
    ]
    invoices = [
        _sanitize_invoice(row)
        for row in _fetch_lnd_invoices(client)
    ]

    forwards = _fetch_lnd_forwarding_history(client)
    import_records = _stamp_lightning_import_network(
        lnd_import_records(invoices, payments, forwards), node_network
    )
    import_outcome = core_imports.insert_wallet_records(
        conn,
        profile,
        wallet,
        import_records,
        LND_IMPORT_SOURCE,
        hooks,
        commit=False,
    )

    # Channel funding/closing txids, so a separately synced on-chain wallet's
    # channel opens/closes net as non-taxable intra-node moves (parity with CLN).
    open_channels = [
        row
        for row in (client.get("/v1/channels").get("channels") or [])
        if isinstance(row, dict)
    ]
    closed_channels = [
        row
        for row in (client.get("/v1/channels/closed").get("channels") or [])
        if isinstance(row, dict)
    ]
    channel_records = lnd_channel_records(
        open_channels,
        closed_channels,
        network=node_network,
    )
    _persist_lnd_channel_records(conn, profile, wallet, backend, channel_records, now_iso())

    invalidate_journals(conn, profile["id"])
    if commit:
        conn.commit()
    return {
        "wallet": wallet["label"],
        "backend": backend["name"],
        "backend_kind": "lnd",
        "status": "synced",
        "transactions": {
            "fetched": len(import_records),
            "imported": import_outcome["imported"],
            "skipped": import_outcome["skipped"],
        },
        "channels": {"records": len(channel_records)},
        "routing": {"forward_events_aggregated": len(forwards)},
    }


# Singleton instance — the registry is keyed by kind and this module is
# only imported when the daemon wires Lightning adapters.
_ADAPTER = LndAdapter()
register_adapter(LndAdapter.kind, _ADAPTER)


__all__ = [
    "LndAdapter",
    "LndRestClient",
    "LND_DEFAULT_PAGE_SIZE",
    "LND_DEFAULT_WINDOW_DAYS",
    "LND_MAX_PAGE_SIZE",
]
